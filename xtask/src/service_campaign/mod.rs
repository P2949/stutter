//! Source-bound, resumable qualification of the packaged bounded systemd service.
pub mod manifest;
pub mod model;
pub mod runner;
pub mod store;

pub mod driver;
mod harness;
mod preflight;
pub use driver::{Args, run};

#[cfg(test)]
mod tests;
