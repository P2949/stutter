//! Desktop/session detection helpers for foreground providers.

pub(crate) fn wayland_session_detected() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

pub(crate) fn desktop_name_lowercase() -> String {
    [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
        "GDMSESSION",
    ]
    .iter()
    .filter_map(|name| std::env::var(name).ok())
    .collect::<Vec<_>>()
    .join(":")
    .to_ascii_lowercase()
}

pub(crate) fn desktop_looks_like_gnome() -> bool {
    desktop_name_lowercase().contains("gnome")
}

pub(crate) fn desktop_looks_like_kde() -> bool {
    let desktop = desktop_name_lowercase();
    desktop.contains("kde")
        || desktop.contains("plasma")
        || std::env::var("KDE_FULL_SESSION").is_ok()
}
