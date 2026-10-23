//! Calibration eligibility of a fixture: the explicit evidence declaration a capture needs before
//! it may satisfy a diagnosis-calibration floor. It reuses the coverage evidence model.
use super::{EvidenceKind, FixtureTomlSummary, REQUIRED_PROVENANCE};

/// Allowed `[provenance].control_type` values for calibration evidence.
pub(crate) const CALIBRATION_CONTROL_TYPES: [&str; 3] =
    ["positive", "negative_control", "mixed_confounded"];

/// What a capture explicitly declares for calibration; present only when it is eligible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalibrationMeta {
    pub kind: EvidenceKind,
    pub control_type: String,
    /// `[coverage].diagnoses`: the families the capture was intentionally designed to exercise.
    pub diagnoses: Vec<String>,
    pub capture_id: String,
}

impl FixtureTomlSummary {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn provenance_value(&self, key: &str) -> &str {
        self.provenance.get(key).map_or("", String::as_str)
    }
    pub(crate) fn primary_cause(&self) -> &str {
        self.primary_cause.as_deref().unwrap_or("")
    }
    /// Legacy name/source inference of "real"; still used for regression sweeps, never for
    /// calibration sufficiency.
    pub(crate) fn is_legacy_real(&self) -> bool {
        self.name.starts_with("real_")
            || self.source == "sanitized-real-recording"
            || self.source == "validation-corpus"
    }

    /// Eligibility for a calibration evidence floor, reusing the coverage evidence model: an
    /// explicit non-synthetic `evidence_kind`, complete `[provenance]`, an explicit
    /// `control_type`, and an explicit `[coverage].diagnoses`. `Err` explains why not.
    pub(crate) fn calibration_meta(&self) -> Result<CalibrationMeta, String> {
        let kind = self
            .evidence_kind
            .as_deref()
            .ok_or("no explicit evidence_kind (legacy inferred evidence)")?;
        let kind = EvidenceKind::parse(kind).map_err(|error| format!("{error:#}"))?;
        if kind == EvidenceKind::Synthetic {
            return Err("explicit evidence_kind is synthetic".to_owned());
        }
        if let Some(key) = REQUIRED_PROVENANCE
            .iter()
            .find(|key| !self.provenance.contains_key(**key))
        {
            return Err(format!("[provenance].{key} is missing"));
        }
        let control_type = self.provenance["control_type"].as_str();
        if !CALIBRATION_CONTROL_TYPES.contains(&control_type) {
            return Err(format!(
                "[provenance].control_type {control_type:?} is not one of {CALIBRATION_CONTROL_TYPES:?}"
            ));
        }
        if self.coverage.diagnoses.is_empty() {
            return Err("[coverage].diagnoses is missing".to_owned());
        }
        Ok(CalibrationMeta {
            kind,
            control_type: control_type.to_owned(),
            diagnoses: self.coverage.diagnoses.clone(),
            capture_id: self.provenance["capture_id"].clone(),
        })
    }
}
