#![allow(dead_code)] // Transitional parse extraction target for Ananicy-compatible input.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommunityRuleImportFormat {
    AnanicyRules,
}
