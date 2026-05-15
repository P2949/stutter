# Generic Tarball Layout

The generic tarball should contain the paths listed in `MANIFEST.txt` relative
to the archive root. Installers can copy the tree under `/usr/local` or another
prefix, then run:

```sh
stutter service doctor --mode system-observe --manager systemd-system
stutter service install --dry-run --mode system-observe --manager systemd-system
```

Low-risk apply mode is intentionally opt-in:

```sh
stutter service install --dry-run --mode system-low-risk --manager systemd-system
stutter daemon restore --dry-run
```

The service command does not enable services automatically. It installs or
removes unit files and prints the explicit follow-up command for the selected
service manager.
