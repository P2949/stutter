//! Foreground, focus, event warning, timing, and wake-graph report tests.

use super::*;
use crate::{
    autotune::state::SituationKind,
    recorder::{FocusEvent, RecordedConfig, SessionMetadataCore},
    sched_state::classify_switch_prev_state,
};

mod display_timing;

mod display_path;

mod foreground;

mod focus;

mod event_stream;

mod switch_state;

mod wake_graph;
