//! StutterScore evidence across a broader workload matrix (post-v0.1). The score's weights are
//! not touched by this study; it records whether one scalar preserves the ordering of the direct
//! evidence across distinct workloads.
pub mod analyze;
pub mod driver;
pub mod model;

pub use driver::{MatrixArgs, run};

#[cfg(test)]
mod tests;
