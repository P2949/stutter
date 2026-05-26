use super::{
    diagnostics::{
        diagnostic_raw_score_delta_f64, diagnostic_raw_score_delta_i64, normalize_owned_reason,
    },
    key::CandidateContextHashInput,
    model::{
        CandidateMemory, CandidateMemoryRecord, CandidateMemoryResult, CandidateResultRecordInput,
        WorkloadActionMemory,
    },
};
use crate::actions::ActionId;

impl CandidateMemory {
    pub fn record_attempt(
        &mut self,
        candidate: &crate::autotune::planning::candidate::CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        self.record_result(CandidateResultRecordInput {
            candidate,
            context,
            now_unix_nanos,
            result: CandidateMemoryResult::Tried,
            diagnostic_baseline_raw_score_total: None,
            diagnostic_current_raw_score_total: None,
            rollback_reason: None,
            cooldown_expires_unix_nanos,
        })
    }

    pub fn record_result(
        &mut self,
        input: CandidateResultRecordInput<'_>,
    ) -> CandidateMemoryRecord {
        let candidate = input.candidate;
        let context = input.context;
        let diagnostic_baseline_raw_score_total = input.diagnostic_baseline_raw_score_total;
        let diagnostic_current_raw_score_total = input.diagnostic_current_raw_score_total;
        let context_key = context.context_key();
        let identity_summary = context.identity_summary();
        let action_id = candidate.action_id();
        let record = CandidateMemoryRecord {
            action_id: action_id.clone(),
            candidate_name: candidate.candidate_name().to_owned(),
            last_tried_unix_nanos: input.now_unix_nanos,
            result: input.result,
            context_hash: context_key.clone(),
            identity_summary: Some(identity_summary),
            degraded_reason: None,
            score_delta: diagnostic_raw_score_delta_i64(
                diagnostic_baseline_raw_score_total,
                diagnostic_current_raw_score_total,
            ),
            rollback_reason: normalize_owned_reason(input.rollback_reason),
            cooldown_expires_unix_nanos: input.cooldown_expires_unix_nanos,
        };

        if let Some(existing) = self.records.iter_mut().find(|existing| {
            existing.action_id == action_id && existing.context_hash == context_key
        }) {
            *existing = record.clone();
        } else {
            self.records.push(record.clone());
        }

        self.record_workload_action(
            candidate,
            context,
            &record,
            diagnostic_baseline_raw_score_total,
            diagnostic_current_raw_score_total,
        );
        self.enforce_bounds();

        record
    }

    pub fn latest(&self) -> Option<&CandidateMemoryRecord> {
        self.records
            .iter()
            .max_by_key(|record| record.last_tried_unix_nanos)
    }

    pub fn last_for_action(&self, action_id: &ActionId) -> Option<&CandidateMemoryRecord> {
        self.records
            .iter()
            .filter(|record| &record.action_id == action_id && !record.is_degraded())
            .max_by_key(|record| record.last_tried_unix_nanos)
    }

    fn record_workload_action(
        &mut self,
        candidate: &crate::autotune::planning::candidate::CandidateAction,
        context: &CandidateContextHashInput,
        record: &CandidateMemoryRecord,
        diagnostic_baseline_raw_score_total: Option<u64>,
        diagnostic_current_raw_score_total: Option<u64>,
    ) {
        let context = context.clone().normalized();
        let Some(workload_identity) = context.workload_identity.clone() else {
            return;
        };
        let action_id = candidate.action_id();
        let action_kind = candidate.action_kind().to_owned();
        let score_delta = diagnostic_raw_score_delta_f64(
            diagnostic_baseline_raw_score_total,
            diagnostic_current_raw_score_total,
        )
        .or(Some(0.0));
        let memory = WorkloadActionMemory {
            workload_hash: workload_identity.as_str().to_owned(),
            action_id: action_id.clone(),
            action_kind,
            objective: candidate.objective(),
            situation: context.situation,
            last_result: record.result.clone(),
            score_delta,
            last_seen_unix_nanos: record.last_tried_unix_nanos,
            cooldown_until_unix_nanos: record.cooldown_expires_unix_nanos,
            exe_dev: context
                .target_executable
                .as_ref()
                .map(|fingerprint| fingerprint.dev()),
            exe_ino: context
                .target_executable
                .as_ref()
                .map(|fingerprint| fingerprint.ino()),
            cgroup_path: context.cgroup_path.clone(),
            identity_summary: context.workload_identity_summary(),
            degraded_reason: None,
        };

        if let Some(existing) = self
            .workload_actions
            .iter_mut()
            .find(|existing| existing.action_id == action_id && existing.matches_context(&context))
        {
            *existing = memory;
        } else {
            self.workload_actions.push(memory);
        }
    }

    fn enforce_bounds(&mut self) {
        const MAX_RECORDS: usize = 512;
        const MAX_WORKLOAD_ACTIONS: usize = 512;

        if self.records.len() > MAX_RECORDS {
            self.records
                .sort_by_key(|record| std::cmp::Reverse(record.last_tried_unix_nanos));
            self.records.truncate(MAX_RECORDS);
        }
        if self.workload_actions.len() > MAX_WORKLOAD_ACTIONS {
            self.workload_actions
                .sort_by_key(|memory| std::cmp::Reverse(memory.last_seen_unix_nanos));
            self.workload_actions.truncate(MAX_WORKLOAD_ACTIONS);
        }
    }
}
