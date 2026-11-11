/* Stutter present-event layer: an opt-in Vulkan layer that stamps each application present call.
 *
 * What it measures: the CLOCK_MONOTONIC time at which the application's vkQueuePresentKHR call
 * ENTERS this layer (an application present SUBMISSION). It is not a completion, scan-out or photon
 * time, and the consumer must never label it as one. It patches no driver and needs no privilege.
 *
 * Enabled only when STUTTER_FRAME_SOCKET names an owner-only Unix stream socket that a controller
 * listens on. Wire format (one JSON object per line):
 *   hello: {"version":1,"epoch":E,"first_seq":0,"event":"present","clock":"CLOCK_MONOTONIC"}
 *   frame: {"seq":N,"monotonic_ns":T}
 * The epoch is the CLOCK_MONOTONIC time at the layer's first present, so a restarted renderer always
 * announces a higher epoch. The controller authenticates the producer from the kernel's peer
 * credentials and the process start time, never from this stream.
 *
 * Limits (by design, not hidden): one VkInstance and one VkDevice per process; if the socket write
 * would block or fails, the layer stops emitting (it never stalls the renderer) and the consumer
 * sees the sequence gap or silence as lost frames. Multi-window and multi-GPU opt-in are separate work. */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#include <vulkan/vk_layer.h>
#include <vulkan/vulkan.h>

#define EXPORT __attribute__((visibility("default")))

static PFN_vkGetInstanceProcAddr next_gipa;
static PFN_vkGetDeviceProcAddr next_gdpa;
static PFN_vkQueuePresentKHR next_present;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static int sock = -1;
static int failed;
static unsigned long long seq;

static unsigned long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (unsigned long long)ts.tv_sec * 1000000000ull + (unsigned long long)ts.tv_nsec;
}

static void send_line(const char *line, size_t len) {
    ssize_t n = send(sock, line, len, MSG_NOSIGNAL | MSG_DONTWAIT);
    if (n != (ssize_t)len) {
        failed = 1; /* never stall or retry: the gap is the signal */
        close(sock);
        sock = -1;
    }
}

static void connect_once(void) {
    const char *path = getenv("STUTTER_FRAME_SOCKET");
    if (!path || !*path || failed || sock >= 0) return;
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    if (strlen(path) >= sizeof addr.sun_path) { failed = 1; return; }
    strcpy(addr.sun_path, path);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) { failed = 1; return; }
    if (connect(fd, (struct sockaddr *)&addr, sizeof addr) != 0) { close(fd); failed = 1; return; }
    sock = fd;
    char hello[200];
    int n = snprintf(hello, sizeof hello,
        "{\"version\":1,\"epoch\":%llu,\"first_seq\":0,\"event\":\"present\",\"clock\":\"CLOCK_MONOTONIC\"}\n",
        now_ns());
    send_line(hello, (size_t)n);
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_present(VkQueue queue, const VkPresentInfoKHR *info) {
    unsigned long long stamp = now_ns();
    pthread_mutex_lock(&lock);
    connect_once();
    if (sock >= 0) {
        char line[96];
        int n = snprintf(line, sizeof line, "{\"seq\":%llu,\"monotonic_ns\":%llu}\n", seq, stamp);
        seq++;
        send_line(line, (size_t)n);
    }
    pthread_mutex_unlock(&lock);
    return next_present(queue, info);
}

static VkLayerInstanceCreateInfo *find_instance_link(const VkInstanceCreateInfo *ci) {
    VkLayerInstanceCreateInfo *p = (VkLayerInstanceCreateInfo *)ci->pNext;
    while (p && !(p->sType == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO && p->function == VK_LAYER_LINK_INFO))
        p = (VkLayerInstanceCreateInfo *)p->pNext;
    return p;
}

static VkLayerDeviceCreateInfo *find_device_link(const VkDeviceCreateInfo *ci) {
    VkLayerDeviceCreateInfo *p = (VkLayerDeviceCreateInfo *)ci->pNext;
    while (p && !(p->sType == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO && p->function == VK_LAYER_LINK_INFO))
        p = (VkLayerDeviceCreateInfo *)p->pNext;
    return p;
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_create_instance(const VkInstanceCreateInfo *ci,
        const VkAllocationCallbacks *alloc, VkInstance *instance) {
    VkLayerInstanceCreateInfo *link = find_instance_link(ci);
    if (!link) return VK_ERROR_INITIALIZATION_FAILED;
    next_gipa = link->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    link->u.pLayerInfo = link->u.pLayerInfo->pNext;
    PFN_vkCreateInstance create = (PFN_vkCreateInstance)next_gipa(NULL, "vkCreateInstance");
    return create(ci, alloc, instance);
}

static VKAPI_ATTR VkResult VKAPI_CALL layer_create_device(VkPhysicalDevice pd, const VkDeviceCreateInfo *ci,
        const VkAllocationCallbacks *alloc, VkDevice *device) {
    VkLayerDeviceCreateInfo *link = find_device_link(ci);
    if (!link) return VK_ERROR_INITIALIZATION_FAILED;
    PFN_vkGetInstanceProcAddr gipa = link->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    next_gdpa = link->u.pLayerInfo->pfnNextGetDeviceProcAddr;
    link->u.pLayerInfo = link->u.pLayerInfo->pNext;
    PFN_vkCreateDevice create = (PFN_vkCreateDevice)gipa(NULL, "vkCreateDevice");
    VkResult r = create(pd, ci, alloc, device);
    if (r == VK_SUCCESS) next_present = (PFN_vkQueuePresentKHR)next_gdpa(*device, "vkQueuePresentKHR");
    return r;
}

EXPORT VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL stutter_layer_gdpa(VkDevice device, const char *name) {
    if (!strcmp(name, "vkQueuePresentKHR")) return (PFN_vkVoidFunction)layer_present;
    if (!strcmp(name, "vkCreateDevice")) return (PFN_vkVoidFunction)layer_create_device;
    return next_gdpa ? next_gdpa(device, name) : NULL;
}

EXPORT VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL stutter_layer_gipa(VkInstance instance, const char *name) {
    if (!strcmp(name, "vkCreateInstance")) return (PFN_vkVoidFunction)layer_create_instance;
    if (!strcmp(name, "vkCreateDevice")) return (PFN_vkVoidFunction)layer_create_device;
    if (!strcmp(name, "vkGetDeviceProcAddr")) return (PFN_vkVoidFunction)stutter_layer_gdpa;
    if (!strcmp(name, "vkQueuePresentKHR")) return (PFN_vkVoidFunction)layer_present;
    return next_gipa ? next_gipa(instance, name) : NULL;
}

EXPORT VKAPI_ATTR VkResult VKAPI_CALL vkNegotiateLoaderLayerInterfaceVersion(VkNegotiateLayerInterface *s) {
    if (s->loaderLayerInterfaceVersion < 2) return VK_ERROR_INITIALIZATION_FAILED;
    s->loaderLayerInterfaceVersion = 2;
    s->pfnGetInstanceProcAddr = stutter_layer_gipa;
    s->pfnGetDeviceProcAddr = stutter_layer_gdpa;
    s->pfnGetPhysicalDeviceProcAddr = NULL;
    return VK_SUCCESS;
}
