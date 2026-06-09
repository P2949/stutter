//! Foreground provider auto-selection.
//!
//! Owns environment-based provider selection and generic Wayland unsupported-session detection. Does
//! not own compositor-specific provider sampling or parser details.

use super::{
    desktop::{desktop_looks_like_gnome, desktop_looks_like_kde},
    gnome::GnomeForegroundProvider,
    hyprland::HyprlandForegroundProvider,
    kde::KdeForegroundProvider,
    sway::SwayForegroundProvider,
    x11::X11ForegroundProvider,
};
use crate::foreground::{
    provider::{ForegroundProvider, UnsupportedForegroundProvider},
    resolver::ForegroundResolver,
};

pub fn auto_foreground_provider() -> Box<dyn ForegroundProvider + Send> {
    if SwayForegroundProvider::is_detected() {
        return Box::new(SwayForegroundProvider::new());
    }

    if HyprlandForegroundProvider::is_detected() {
        return Box::new(HyprlandForegroundProvider::new());
    }

    if GnomeForegroundProvider::is_detected() {
        return Box::new(GnomeForegroundProvider::new());
    }

    if KdeForegroundProvider::is_detected() {
        return Box::new(KdeForegroundProvider::new());
    }

    if is_generic_wayland_without_supported_foreground_api() {
        if desktop_looks_like_gnome() {
            return Box::new(UnsupportedForegroundProvider::new(
                "GNOME Wayland session detected, but stutter-gnome-foreground is required; unsafe org.gnome.Shell Eval is intentionally not used",
            ));
        }

        if desktop_looks_like_kde() {
            return Box::new(UnsupportedForegroundProvider::new(
                "KDE Wayland session detected, but stutter-kde-foreground is required; KWin script injection is intentionally not used",
            ));
        }

        return Box::new(UnsupportedForegroundProvider::generic_wayland());
    }

    if X11ForegroundProvider::is_detected() {
        return Box::new(X11ForegroundProvider::new());
    }

    Box::new(UnsupportedForegroundProvider::new(
        "no supported foreground-window provider detected",
    ))
}

pub fn auto_foreground_resolver() -> ForegroundResolver {
    ForegroundResolver::new(auto_foreground_provider())
}

pub(crate) fn is_generic_wayland_without_supported_foreground_api() -> bool {
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        return false;
    }

    if std::env::var("SWAYSOCK").is_ok() {
        return false;
    }

    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        return false;
    }

    true
}

#[cfg(test)]
pub(crate) fn current_desktop_looks_like_gnome_or_kde() -> bool {
    desktop_looks_like_gnome() || desktop_looks_like_kde()
}
