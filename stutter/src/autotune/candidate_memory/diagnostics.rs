use super::model::CandidateMemory;

impl CandidateMemory {
    pub fn degraded_diagnostics(&self) -> Vec<String> {
        let mut diagnostics = Vec::new();

        for record in &self.records {
            if let Some(reason) = record.degraded_reason.as_ref() {
                diagnostics.push(format!(
                    "record action_id={} candidate={} context_hash={} reason={}",
                    record.action_id.as_str(),
                    record.candidate_name,
                    record.context_hash.as_str(),
                    reason
                ));
            }
        }

        for memory in &self.workload_actions {
            if let Some(reason) = memory.degraded_reason.as_ref() {
                diagnostics.push(format!(
                    "workload_action action_id={} workload_hash={} reason={}",
                    memory.action_id.as_str(),
                    memory.workload_hash,
                    reason
                ));
            }
        }

        diagnostics
    }
}

pub(super) fn normalize_owned_reason(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Computes a raw total-score delta for persisted diagnostics.
pub(super) fn diagnostic_raw_score_delta_i64(
    diagnostic_baseline_raw_score_total: Option<u64>,
    diagnostic_current_raw_score_total: Option<u64>,
) -> i64 {
    let (Some(baseline), Some(current)) = (
        diagnostic_baseline_raw_score_total,
        diagnostic_current_raw_score_total,
    ) else {
        return 0;
    };

    let delta = current as i128 - baseline as i128;
    if delta > i64::MAX as i128 {
        i64::MAX
    } else if delta < i64::MIN as i128 {
        i64::MIN
    } else {
        delta as i64
    }
}

/// Computes a raw total-score delta for workload memory summaries.
pub(super) fn diagnostic_raw_score_delta_f64(
    diagnostic_baseline_raw_score_total: Option<u64>,
    diagnostic_current_raw_score_total: Option<u64>,
) -> Option<f64> {
    let (Some(baseline), Some(current)) = (
        diagnostic_baseline_raw_score_total,
        diagnostic_current_raw_score_total,
    ) else {
        return None;
    };

    Some(current as f64 - baseline as f64)
}
