use std::{
    path::{Path, PathBuf},
    process::Command,
};

const FOREGROUND_HELPER_PATH: &str = "/usr/bin:/bin";
const TRUSTED_FOREGROUND_HELPER_DIRS: &[&str] = &["/usr/bin", "/bin"];
const FOREGROUND_HELPER_ENV_ALLOWLIST: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "GDMSESSION",
    "GNOME_SHELL_SESSION_MODE",
    "HYPRLAND_INSTANCE_SIGNATURE",
    "KDE_FULL_SESSION",
    "SWAYSOCK",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_DESKTOP",
];

pub(crate) fn resolve_trusted_foreground_helper(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    if program.contains('/') {
        return None;
    }

    TRUSTED_FOREGROUND_HELPER_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

pub(crate) fn trusted_foreground_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    command.env("PATH", FOREGROUND_HELPER_PATH);
    for name in FOREGROUND_HELPER_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}
