pub mod effective;
pub mod layer;
pub mod merge;
pub mod model;
pub mod schema;
pub mod source;
pub mod types;
pub(crate) mod validation;

pub use types::{
    CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX, WaylandPresentationSource,
};

pub use crate::error::ConfigError;
