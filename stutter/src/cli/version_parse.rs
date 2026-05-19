#![allow(dead_code)] // Transitional CLI split: version parsing moves here incrementally.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestedVersionFeature {
    pub feature: String,
}
