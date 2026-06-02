//! Diagnosis assembly and public re-exports; implementation lives in focused child modules.

mod anchor;
mod candidate;
mod evidence;
mod evidence_chain;
mod model;
mod orchestrator;

pub(crate) use anchor::{select_anchor, select_anchor_for_diagnosis};
pub use model::{
    CandidateRejection, ClusterAnchor, ClusterAnchorKind, Confidence, Diagnosis,
    DiagnosisCandidate, DiagnosisConfig, DiagnosisThresholdDoc, EvidenceChain, EvidenceChainKind,
    EvidenceChainNode, EvidenceChainNodeKind, EvidenceItem, EvidenceKind, FrameDiagnosis,
    LiveDiagnosisEntry, StutterCause,
};
pub(crate) use orchestrator::diagnose_cluster;
#[cfg(test)]
pub(crate) use orchestrator::diagnose_cluster_with_config;

#[cfg(test)]
mod tests;
