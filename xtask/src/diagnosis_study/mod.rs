//! Post-v0.1 diagnosis evidence study: matched controls, independent intervention and
//! target-impact proof, informational calibration labels. A resumable driver reuses the
//! scenario/capture machinery and the privileged runner; it does not replicate the campaign.
pub mod calibrate;
pub mod capture;
pub mod driver;
pub mod facts;
pub mod impact;
pub mod inspect;
pub mod metric;
pub mod model;
pub mod plan;
pub mod promote;
pub mod store;

pub use driver::{StudyArgs, run};

#[cfg(test)]
mod tests;
