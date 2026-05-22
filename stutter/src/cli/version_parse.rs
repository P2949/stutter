#![allow(dead_code)] // Transitional CLI split: version parsing moves here incrementally.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestedVersionFeature {
    pub feature: String,
}
