//! Sway foreground provider and parser tests extracted from `foreground`.
//!
//! Owns Sway tree parsing, Sway detection, Sway Wayland detection, and Sway title-redaction provider tests.
//! Does not own resolver policy, X11 parsing, Hyprland parsing, or production foreground behavior.

use super::{super::*, SequenceProvider, restore_env_var};

mod tree;

mod focus;

mod redaction;

mod malformed;

mod workspace;
