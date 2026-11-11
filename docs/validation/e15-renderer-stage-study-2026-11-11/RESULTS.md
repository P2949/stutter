# E15: observational Vulkan application-stage study: BlockedResource (2026-11-11)

Question (PLAN.md E15): under a controlled `vkcube` with one opt-in layer, do `vkQueuePresentKHR`-entry intervals respond to bounded owned background CPU
placement, with identity and no gaps retained across a short baseline/treatment comparison?

Disposition: **BlockedResource for this stage**, with the completed W21 transport proof preserved. Reason, from the host and the code:
- `vkcube --help` on this host lists `--wsi <xcb|xlib|wayland|display>` only. Every option presents into a real window on the active Wayland
  session (xcb is Xwayland) or takes the display directly (`display` needs DRM master, forbidden). There is no headless presenting mode, so every run
  of the study would map a window on the user's occupied graphical session; PLAN.md E15 allows the run only if the renderer can launch and exit
  without disrupting the current desktop and forbids any session takeover.
- The Vulkan instance does list `VK_EXT_headless_surface`, but `vkcube` cannot use it, and a purpose-built headless presenting producer is a new
  native program (a swapchain-less present path does not exercise `vkQueuePresentKHR` the way the layer stamps it), which is not a bounded change to
  the existing layer and would be a new, unreviewed renderer rather than the unmodified `vkcube` the plan specifies.
- The read-only capability probe that does not need a window was run (E6): `VK_EXT_present_timing` is exposed by RADV with an
  `IMAGE_FIRST_PIXEL_OUT` stage on native Wayland surfaces; it is recorded as a discovered capability, not built into a collector, and is not
  independent optical proof. `stutter wayland-probe` opens a client surface (default 30 s, optional fullscreen) and was not run for the same reason.

Preserved and unchanged: the 2,400-event W21 proof (0 gaps, p99 delivery 82 us, wrong-process refusal, keep denied), the 2 ms Direct gate and
`GameFramePacing` observation-only. No layer or producer code changed, no worker token, no GPU/LACT or electrical setting touched, no hardware or
permission request. If a headless presenting path or a window-free present surface appears later, this study can be frozen and run then.
