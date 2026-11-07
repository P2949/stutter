//! Workload identity and frame-source provenance (package B).
//!
//! A frame stream may drive a frame-primary decision only when its producer is *verified* to
//! belong to the one target workload. Every mismatch is a typed denial, never an assumed match.
//! Launch identifiers are correlation data and never privileged authority, and this module has no
//! mutation path.
pub mod frame_source;
pub mod identity;
pub mod live_source;
pub mod proc_view;
pub mod quality;
pub mod registration;
pub mod session;

#[cfg(test)]
mod tests;
