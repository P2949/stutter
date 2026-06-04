# KCD1 real-world stack comparison

This is an exploratory follow-up to the controlled KCD1 case study.

It compares:
- clean-stack: stripped-down measurement launch configuration, 1080p/100Hz/100FPS cap, default scheduler.
- personal-stack: author's usual gaming launch configuration plus scx_lavd aggressive scheduler settings.

This is not a causal test of any individual flag. The personal-stack condition changes multiple variables at once, including scheduler, Gamescope mode, resolution/output scaling, RADV/Mesa options, Wine/Proton options, allocator/gamemode, and presentation behavior.
