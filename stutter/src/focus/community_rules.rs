#![allow(dead_code)] // Transitional focus/community-rules integration target.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommunityRuleFocusHint {
    pub process_name: String,
    pub class_label: String,
}
