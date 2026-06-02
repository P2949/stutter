//! Foreground provider implementations grouped by selection mode.
//!
//! Owns provider module wiring. Does not own model DTOs, parser modules, or resolver policy.

pub(crate) mod auto;
pub(crate) mod desktop;
pub(crate) mod gnome;
pub(crate) mod hyprland;
pub(crate) mod kde;
pub(crate) mod sway;
pub(crate) mod x11;
