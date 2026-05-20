//! Foreground-window detection, parsing, and stale-snapshot resolution.
//!
//! Owns foreground provider module wiring and the compatibility import surface used by session,
//! focus, recorder, and report code. Implementation details live in the child modules by model,
//! provider, parser, and resolver responsibility.

pub(crate) mod command;
pub(crate) mod model;
pub(crate) mod parse;
pub(crate) mod provider;
pub(crate) mod providers;
pub(crate) mod resolver;

pub use model::{
    DEFAULT_FOREGROUND_POLL_MS, ForegroundAvailableInput, ForegroundEvent, ForegroundEventInput,
    ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot,
};
#[cfg(test)]
pub(crate) use parse::x11::parse_x11_quoted_strings;
#[cfg(test)]
pub(crate) use provider::ForegroundProvider;
#[cfg(test)]
pub(crate) use provider::{GENERIC_WAYLAND_UNSUPPORTED_REASON, UnsupportedForegroundProvider};
#[cfg(test)]
pub(crate) use providers::auto::{
    current_desktop_looks_like_gnome_or_kde, is_generic_wayland_without_supported_foreground_api,
};
#[cfg(test)]
pub(crate) use providers::{
    auto::auto_foreground_provider, hyprland::hyprland_snapshot_from_activewindow_json,
};
pub(crate) use providers::{
    auto::auto_foreground_resolver, hyprland::HyprlandForegroundProvider,
    sway::SwayForegroundProvider, x11::X11ForegroundProvider,
};
pub(crate) use resolver::ForegroundResolver;

#[cfg(test)]
#[path = "foreground/tests/mod.rs"]
mod tests;
