//! Workload identity and frame-source provenance (package B).
//!
//! A frame stream may drive a frame-primary decision only when its producer is *verified* to
//! belong to the one target workload. Every mismatch is a typed denial, never an assumed match.
//! Launch identifiers are correlation data and never privileged authority, and this module has no
//! mutation path.
pub mod frame_clock;
pub mod frame_source;
pub mod gamescope;
pub mod identity;
pub mod launch_discovery;
pub mod live_source;
pub mod managed_source;
pub mod proc_view;
pub mod quality;
pub mod registration;
pub mod renderers;
pub mod session;
pub mod stamped_frames;
pub mod stamped_source;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod launch_tests;
