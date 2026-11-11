# W21: a real instrumented renderer reaches the authenticated stamped source (2026-11-11)

**Producer.** `packaging/vulkan-layer/stutter_present_layer.c` (sha256 `ac3e82baea296caab9cabf686ca57109bf89ab622346022d3195c15f60e52be7`), an opt-in Vulkan layer
(`VK_LAYER_STUTTER_present`; no driver patch, no privilege). It stamps `CLOCK_MONOTONIC` when the application's
`vkQueuePresentKHR` is **entered**: an application present *submission*, never scan-out. It connects to an owner-only Unix
socket named by `STUTTER_FRAME_SOCKET`, announces protocol 1, an epoch (its first-present time, so a restarted renderer always
announces a higher one), `first_seq` 0 and event `present`, then writes `{"seq","monotonic_ns"}` lines. It never blocks the
renderer: a failed or would-block write stops emission, and the consumer sees the sequence gap or silence.

**Renderer.** Unmodified `vkcube` (Wayland, RADV on this machine's GPU) with the layer loaded through `VK_ADD_LAYER_PATH` and
`VK_INSTANCE_LAYERS`.

**Consumer.** `xtask renderer-producer` listens in a fresh `0700` directory and binds through `StampedSource::accept`: kernel
peer credentials (uid and pid), the process start time, the shared monotonic time namespace, a known protocol/clock/event and a
higher epoch. Result (`result.json`, 2400 frames):

* 2400 accepted, 0 rejected, 0 malformed, 0 unexplained gaps; delivery over 1200 frames: p99 82 µs, max 2.16 ms.
* Midstream (renderer alive): attribution `ValidatedDirect`, stage `ApplicationFrame`, source healthy, 0 dropped.
* The same connection under a policy naming another pid: `Err(WrongProcess)`.
* After the renderer exits: attribution `Unavailable`, source unhealthy (identity is rechecked, so a dead renderer is not believed).
* **Direct-frame keep: denied (`AlignmentUncertain`).** The alignment uncertainty is unknown because there is no independent
  stamp-accuracy reference (W22). This is the required outcome: no direct-frame keep unless W22 passes.

**Limits stated, not hidden.** One VkInstance and one VkDevice per process; Proton/Wine, Gamescope layering, multi-window and
multi-GPU opt-in are separate work and are not claimed. The producer was not exercised against a game.

**W22 (independent stamp accuracy).** The accepted references are a photodiode, a display-hardware timestamp or a
high-speed camera. None exists on this machine, and a display hardware timestamp for a composited window would require taking the
display away from the desktop session. This is therefore recorded as `NotPlannedForVerification` (different hardware); the
frame objective stays measurement-only.
