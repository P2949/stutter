//! Intentional public API façade for the `stutter` crate.
//!
//! Root subsystem modules are kept crate-private. Public consumers should use
//! this module, plus the root `run_cli` entry point and root `StutterError`
//! re-export, instead of depending on internal module layout.

pub mod error {
    //! Public error types returned by stable crate entry points.

    pub use crate::error::StutterError;
}
