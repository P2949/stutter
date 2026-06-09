use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnosis {
    pub primary: Option<DiagnosisPrimary>,
    pub candidates: Vec<DiagnosisCandidate>,
    pub missing_evidence: Vec<String>,
    #[serde(default)]
    pub evidence_chains: Vec<DiagnosisEvidenceChain>,
    pub candidate_rejections: Vec<DiagnosisRejection>,
    pub secondary_causes: Vec<String>,
    pub report_summary: String,
}

impl Diagnosis {
    pub fn report_summary(&self) -> &str {
        &self.report_summary
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisPrimary {
    pub cause: String,
    pub confidence: String,
    pub score: f32,
    pub evidence: Vec<DiagnosisEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisCandidate {
    pub cause: String,
    pub confidence: String,
    pub score: f32,
    pub evidence: Vec<DiagnosisEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisRejection {
    pub cause: String,
    pub score: f32,
    pub confidence: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisEvidence {
    pub kind: String,
    pub strength: f32,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisEvidenceChain {
    pub kind: String,
    pub explicit: bool,
    pub summary: String,
    pub nodes: Vec<DiagnosisEvidenceChainNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisEvidenceChainNode {
    pub kind: String,
    pub label: String,
    pub timestamp_ms: Option<u64>,
    pub start_ns: Option<u64>,
    pub end_ns: Option<u64>,
    pub delta_from_previous_ms: Option<i64>,
    #[serde(default)]
    pub details: std::collections::BTreeMap<String, String>,
}
