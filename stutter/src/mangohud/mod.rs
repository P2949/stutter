use std::{fs, path::Path, time::Duration};

use anyhow::Context;

mod alignment;
mod parser;
mod plausibility;
mod schema;
mod tail;
mod tail_session;

pub use alignment::poll_alignment;
pub use parser::read_frame_events;
#[cfg(test)]
use parser::{MangoHudLiveParser, parse_frame_events};
#[cfg(test)]
use schema::{
    ElapsedUnit, detect_layout, detect_layout_from_reader, schema_from_headers, split_csv_line,
};
pub use tail::tail_frames;
pub use tail_session::{
    MangoHudTailSession, TailConfig, TailCounters, TailFailure, TailState, TailedFrame,
};
#[cfg(test)]
mod tests;
