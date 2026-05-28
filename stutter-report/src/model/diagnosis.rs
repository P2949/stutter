use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnosis {
    pub primary: Option<DiagnosisPrimary>,
    pub candidates: Vec<DiagnosisCandidate>,
    pub missing_evidence: Vec<String>,
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
