# KCD1 real-world stack launch options

This file preserves the exact launch/scheduler setup used for the exploratory
real-world stack comparison. The comparison is a configuration-bundle check, not
a causal test of any individual flag.

## Clean stack

Purpose: stripped-down measurement setup used as the controlled reference for the
real-world stack add-on.

Properties:

- default scheduler / `sched_ext` disabled
- Gamescope at 1920x1080
- 100 Hz output
- 100 FPS MangoHud cap
- MangoHud logging enabled
- fixed `+exec user.cfg`
- no RADV experimental flags
- no FSR/FSR4
- no gamemode
- no mimalloc
- no forced Wine CPU topology

Steam launch options:

```bash
MESA_SHADER_CACHE_MAX_SIZE=20G DRI_PRIME=pci-0000_03_00_0! VK_LOADER_DRIVERS_SELECT='*radeon*' gamescope -w 1920 -h 1080 -W 1920 -H 1080 -r 100 --force-grab-cursor -f -- env DRI_PRIME=pci-0000_03_00_0! VK_LOADER_DRIVERS_SELECT='*radeon*' MESA_SHADER_CACHE_MAX_SIZE=20G MANGOHUD_CONFIG='autostart_log=1,output_folder=/home/p2949/.local/state/stutter/mangohud-kcd-clean,log_interval=0,log_versioning=1,fps_limit=100' MANGOHUD=1 %command% +exec user.cfg
```

## Personal stack

Purpose: the author's normal player-used gaming configuration bundle. This
changes many variables at once, so it must not be interpreted as a causal test of
`scx_lavd`, Gamescope FSR, RADV/Mesa flags, Wine/Proton flags, allocator choice,
gamemode, or any individual launch flag.

Properties:

- `scx_lavd` enabled with the aggressive gaming flags listed below
- Gamescope internal size 2560x1440 and output size 3840x2160
- 60 Hz output
- Gamescope FSR scaling enabled
- RADV/Mesa experimental/performance flags enabled
- Wine/Proton flags enabled
- mimalloc and gamemode enabled
- MangoHud logging enabled
- fixed `+exec user.cfg`

Steam launch options:

```bash
unset WLR_XWAYLAND; MESA_SHADER_CACHE_MAX_SIZE=20G ENABLE_GAMESCOPE_WSI=1 PULSE_LATENCY_MSEC=40 LD_PRELOAD=/usr/lib64/libmimalloc.so:/usr/lib64/libgamemodeauto.so.0 MESA_VK_ENABLE_SUBMIT_THREAD=1 MESA_DISK_CACHE_DATABASE=1 MESA_NO_ERROR=1 WINE_FULLSCREEN_FSR=1 PROTON_FSR4_UPGRADE=1 FSR4_UPGRADE=1 PROTON_FSR4_INDICATOR=1 ENABLE_LAYER_MESA_ANTI_LAG=1 RADV_PERFTEST=nggc,sam,afmf,nircache,rtcps,localbos,nogttspill RADV_EXPERIMENTAL=transfer_queue,hic,sparse,video_decode RADV_USER_ENABLED_OPTION_STRING=antilag+ RADV_TEX_ANISO=16 gamescope -w 2560 -W 3840 -h 1440 -H 2160 -r 60 -F fsr --synchronous-x11 --sharpness 20 --fsr-sharpness 20 -s 2 --force-grab-cursor --adaptive-sync --rt --immediate-flips -f -- env DRI_PRIME=pci-0000_03_00_0! VK_LOADER_DRIVERS_SELECT='*radeon*' AMD_USERQ=1 AQ_DRM_DEVICES=/dev/dri/by-path/pci-0000:03:00.0-card WLR_DRM_DEVICES=/dev/dri/by-path/pci-0000:03:00.0-card MOZ_DRM_DEVICE=/dev/dri/by-path/pci-0000:03:00.0-card VDPAU_DRIVER=radeonsi LIBVA_DRIVER_NAME=radeonsi MESA_LOADER_DRIVER_OVERRIDE=radeonsi __GLX_VENDOR_LIBRARY_NAME=mesa MESA_NO_ERROR=1 RADV_PERFTEST=nggc,sam,afmf,nircache,rtcps,localbos,nogttspill RADV_EXPERIMENTAL=transfer_queue,hic,sparse,video_decode RADV_USER_ENABLED_OPTION_STRING=antilag+ RADV_TEX_ANISO=16 ENABLE_LAYER_MESA_ANTI_LAG=1 SDL_VIDEODRIVER=wayland WINE_SIMULATE_WRITECOPY=1 WINE_DISABLE_WRITE_WATCH=1 PROTON_USE_NTSYNC=1 PROTON_NO_FSYNC=0 PROTON_NO_ESYNC=0 WINEESYNC=1 WINEFSYNC=1 PROTON_ENABLE_WAYLAND=1 PROTON_ENABLE_AMD_AGS=1 PROTON_FORCE_LARGE_ADDRESS_AWARE=1 PROTON_HEAP_DELAY_FREE=1 PROTON_USE_SECCOMP=1 PROTON_NO_XIM=0 PROTON_EAC_RUNTIME=1 PROTON_BATTLEYE_RUNTIME=1 PROTON_USE_WOW64=1 MESA_DISK_CACHE_DATABASE=1 WINE_CPU_TOPOLOGY=12:0,1,2,3,4,5,6,7,8,9,10,11 VKD3D_CONFIG=dxr,dxr12,force_static_cbv VKD3D_SHADER_MODEL=6_7 MESA_VK_ENABLE_SUBMIT_THREAD=1 PULSE_LATENCY_MSEC=40 VKD3D_FEATURE_LEVEL=12_2 MESA_SHADER_CACHE_MAX_SIZE=20G MANGOHUD_CONFIG='autostart_log=1,output_folder=/home/p2949/.local/state/stutter/mangohud-kcd-personal,log_interval=0,log_versioning=1' MANGOHUD=1 gamemoderun %command% +exec user.cfg
```

## Personal-stack scheduler command

The personal-stack runs used `scx_lavd` with the following flags:

```bash
doas scx_lavd \
  --cpu-pref-order 1,2,3,4,5,0,7,8,9,10,11,6 \
  --performance \
  --preempt-shift 0 \
  --slice-min-us 100 \
  --slice-max-us 1000 \
  --pinned-slice-us 250 \
  --lb-low-util-pct 0 \
  --lb-local-dsq-util-pct 0 \
  --per-cpu-dsq
```

The postcheck artifacts for the personal-stack runs reported `sched_ext` enabled
with `lavd_1.1.0_x86_64_unknown_linux_gnu`. The clean-stack runs were recorded
with `sched_ext` disabled.

## Interpretation note

This add-on intentionally represents a realistic player-used configuration
bundle. It should be used to show that `stutter` can capture and compare a
real-world setup, not to claim that any single scheduler, driver flag, Wine flag,
allocator, or Gamescope option caused the observed result.
