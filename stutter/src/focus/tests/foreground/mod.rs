//! Foreground-aware focus tests extracted from `focus::mod`.
//!
//! Owns foreground-source, foreground-scoring, and foreground resolver sample coverage.
//! Does not own shared fixtures or production focus behavior.

use crate::focus::{
    test_support::{
        foreground_scoring_snapshot, foreground_snapshot, foreground_test_group as test_group,
        foreground_test_process as test_process,
    },
    *,
};

mod classification;

mod safety;

mod stale;

mod scoring;

mod active_window;
