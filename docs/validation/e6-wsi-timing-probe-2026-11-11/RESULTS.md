# E6: WSI presentation-timing capability discovery on this host (2026-11-11)

Scope (PLAN.md E6, W21/W22 optional probe): feature discovery only, on the real RADV/Wayland stack. `vulkaninfo` (full output in
`vulkaninfo-full.txt`, sha256 prefix `287f8921`, stderr in `vulkaninfo.stderr`) was run once, read-only: it enumerates instance,
device and surface capabilities. No swapchain was created, no frame was presented, no window was mapped, no DRM master was taken,
and the session, Stutter units and every setting were untouched. Driver: RADV, Mesa 26.2.6, Vulkan API 1.4.354, GPU0 AMD Radeon
RX 9070 XT (RADV GFX1201); GPU1 is llvmpipe (software, irrelevant to the game GPU).

## What the real path exposes (GPU0, RADV)

| Capability | Result |
|---|---|
| `VK_EXT_present_timing` | present, revision 3; feature `presentTiming = true`, `presentAtAbsoluteTime = true`, `presentAtRelativeTime = true` |
| `VK_KHR_present_id` / `present_id2` | present (`presentId = true`, surface `presentId2Supported = true`) |
| `VK_KHR_present_wait` / `present_wait2` | present (`presentWait2 = true`) |
| `VK_KHR_calibrated_timestamps` / `VK_EXT_calibrated_timestamps` | present |
| `VK_KHR_swapchain_maintenance1` / `VK_EXT_swapchain_maintenance1` | present |
| `VK_GOOGLE_display_timing` | absent (no `VK_GOOGLE_display_timing` anywhere in the output) |

Surface-level present-timing capability differs by windowing path:

| Surface type (RADV) | `presentTimingSupported` | absolute / relative time | stages that can be queried |
|---|---|---|---|
| native Wayland (`VK_KHR_wayland_surface`) | true | absolute yes / relative no | `QUEUE_OPERATIONS_END`, `IMAGE_FIRST_PIXEL_OUT` |
| Xwayland (`VK_KHR_xcb_surface` / `xlib`) | true | absolute no / relative yes | `QUEUE_OPERATIONS_END`, `REQUEST_DEQUEUED` |

## Disposition

- **Upgrade path discovered (PLAN.md section 3.3 item 5), not implemented.** On a native Wayland surface this host's driver advertises
  a read-only `IMAGE_FIRST_PIXEL_OUT` presentation-feedback stage with present IDs and calibrated timestamps. That is a candidate
  stage-specific observation to cross-check against the `vkQueuePresentKHR`-entry stamps. It was NOT built: an unmodified
  application does not request `VK_EXT_present_timing`, so the opt-in layer would have to inject the extension, swapchain flag and
  queue, and PLAN.md forbids a guessed extension loader and requires a bounded, real producer, a calibration against the call-entry
  stamp and a frozen A/B, none of which this capability probe provides. It is recorded as a typed, discovered capability.
- **What it would not be even if built.** The stage is what the driver/compositor reports. Whether `IMAGE_FIRST_PIXEL_OUT` equals
  the moment pixels reach the panel is not established here (no photodiode, camera or second display), and the compositor's own
  presentation-time protocol was not independently checked (`wayland-info` is not installed). So the 2 ms alignment gate stays
  unweakened, `AlignmentUncertain` stays, `GameFramePacing` stays observation-only, and no direct-frame keep follows from this.
- **Honest API-stage result that already exists:** the W21 renderer proof (2,400 `vkQueuePresentKHR`-entry events, 0 gaps, p99
  delivery 82 us, wrong-process refusal), labelled `application_present_submit`. No new frame A/B was run: any Vulkan present on
  this single-GPU Wayland desktop maps a window on the active session, which PLAN.md forbids disturbing for an optional study.
- **Not run:** the optional bounded uprobe/perf trace calibrating call-entry bias (it needs a presenting renderer and tracing
  rights); noted here as not performed, with the same window-on-the-session reason.
