# KCD1 Case Study Method Notes

## Game
- Game: Kingdom Come: Deliverance 1
- Platform: Steam / Proton
- Proton version: GE-Proton10-34
- Launch options: minimal KCD1 measurement launch options; Gamescope + MangoHud logging only; no RADV experimental flags, no FSR/FSR4, no gamemode, no mimalloc, no forced Wine sync/topology. The committed presentation archive keeps the launch shape rather than the raw shell line: Steam -> Gamescope 1920x1080 at 100 Hz -> MangoHud logging with a 100 FPS cap -> GE-Proton10-34 -> `KingdomCome.exe +exec user.cfg`.
- Resolution: 1920x1080 native via Gamescope
- Refresh rate / FPS cap: 100 Hz / 100 FPS cap via MangoHud
- Graphics preset: ultra (technically custom because everything is maxed out, but the game doesn't max out all settings at ultra preset, so it's effectively a custom preset with everything maxed out)
- VSync: off
- Gamescope used? yes/no: yes
- MangoHud used? yes/no: yes 
- Desktop/session: Wayland + Hyprland / other: sway (wayland)

## Route
- Save name/location: playline 2 / Rattay
- Route label: Rattay - KCD1 fixed route 1
- Start point: Rattay entrance. (close to the apothecary)
- End point: Rattay entrance after going through the town (reaching the blacksmith apprentice and then returning to the entrance)
- Duration target: 180 seconds

## Controls
- Shader cache warm-up run done? yes/no: yes
- Browser/Discord/downloads closed? yes/no: yes
- Same save used for all runs? yes/no: yes
- Same graphics settings used for all runs? yes/no: yes
- Same Proton version used for all runs? yes/no: yes
- Same display/FPS cap used for all runs? yes/no: yes

## Tuning hypothesis
A process-tree CPU-affinity profile that separates game/main/render threads from compositor/helper activity reduces scheduler delay and frame-time tail latency during this fixed KCD1 route.

## Notes
- The large personal optimized launch configuration was intentionally not used for the first case study because it changes many variables at once. The experiment keeps the launch environment fixed and only changes the stutter tuning profile during A/B validation.

## KCD1 configuration files

The Steam launch options include `+exec user.cfg`. The contents of `user.cfg` were archived before measurement.

The user configuration mainly sets memory, texture-streaming, material preload, and pak stream-cache options. These settings may affect frame pacing and asset-streaming behavior, so they are treated as part of the fixed workload configuration rather than as a tuning variable.

This means the experiment does not compare stock KCD1 against optimized KCD1. It compares the same fixed KCD1 configuration under baseline scheduler placement and under a scoped `stutter` CPU/thread-placement profile.
