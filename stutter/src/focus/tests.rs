//! Focus test modules split by behavior area.
//!
//! Owns test module wiring only. Test helpers live in `super::test_support`.
//! Does not own production focus behavior.

mod classification;
mod foreground;
mod groups;
mod resolver;
mod safety;
mod scoring;
mod snapshot;
