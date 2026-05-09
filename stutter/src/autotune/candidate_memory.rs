#![cfg(feature = "autotune-controller")]

use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};

use super::{candidate::CandidateAction, observation::AutotuneObservation, state::SituationKind};
use crate::{
    actions::ActionId,
    process_tree::{TargetSnapshot, TaskClass},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateMemoryResult {
    Tried,
    Kept,
    Reverted,
    Faulted,
    Inconclusive,
    Rejected,
}

impl CandidateMemoryResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tried => "tried",
            Self::Kept => "kept",
            Self::Reverted => "reverted",
            Self::Faulted => "faulted",
            Self::Inconclusive => "inconclusive",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateClassCount {
    pub class: TaskClass,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateContextHashInput {
    pub target_exe_dev: Option<u64>,
    pub target_exe_ino: Option<u64>,
    pub cpu_topology_signature: Option<String>,
    pub profile_name: String,
    pub situation: SituationKind,
    pub active_task_class_distribution: Vec<CandidateClassCount>,
}

impl CandidateContextHashInput {
    pub fn for_candidate(candidate: &CandidateAction) -> Self {
        Self {
            target_exe_dev: None,
            target_exe_ino: None,
            cpu_topology_signature: None,
            profile_name: candidate.profile_name().to_owned(),
            situation: SituationKind::Unknown,
            active_task_class_distribution: Vec::new(),
        }
    }

    pub fn from_observation(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
    ) -> Self {
        Self {
            target_exe_dev: None,
            target_exe_ino: None,
            cpu_topology_signature: normalize_optional_string(cpu_topology_signature),
            profile_name: candidate.profile_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: Vec::new(),
        }
    }

    pub fn from_snapshot(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
        snapshot: &TargetSnapshot,
    ) -> Self {
        let (target_exe_dev, target_exe_ino) =
            target_executable_inode_from_snapshot(observation, snapshot);

        Self {
            target_exe_dev,
            target_exe_ino,
            cpu_topology_signature: normalize_optional_string(cpu_topology_signature),
            profile_name: candidate.profile_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: class_distribution_from_snapshot(snapshot),
        }
    }

    pub fn normalized(mut self) -> Self {
        self.cpu_topology_signature = self
            .cpu_topology_signature
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        self.active_task_class_distribution =
            normalize_class_distribution(self.active_task_class_distribution);

        self
    }

    pub fn context_hash(&self) -> String {
        let normalized = self.clone().normalized();
        let mut parts = Vec::new();

        parts.push(format!(
            "target_exe={}:{}",
            normalized
                .target_exe_dev
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            normalized
                .target_exe_ino
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        ));

        parts.push(format!(
            "cpu_topology={}",
            normalized.cpu_topology_signature.as_deref().unwrap_or("-")
        ));
        parts.push(format!("profile={}", normalized.profile_name));
        parts.push(format!("situation={:?}", normalized.situation));

        for count in &normalized.active_task_class_distribution {
            parts.push(format!("class={}:{}", count.class.as_str(), count.count));
        }

        stable_hash_hex(&parts)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateMemoryRecord {
    pub action_id: ActionId,
    pub candidate_name: String,
    pub last_tried_unix_nanos: u128,
    pub result: CandidateMemoryResult,
    pub context_hash: String,
    pub score_delta: i64,
    pub rollback_reason: Option<String>,
    pub cooldown_expires_unix_nanos: Option<u128>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateMemory {
    pub records: Vec<CandidateMemoryRecord>,
}

impl CandidateMemory {
    pub fn record_attempt(
        &mut self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        self.record_result(
            candidate,
            context,
            now_unix_nanos,
            CandidateMemoryResult::Tried,
            None,
            None,
            None,
            cooldown_expires_unix_nanos,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_result(
        &mut self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
        result: CandidateMemoryResult,
        baseline_score_total: Option<u64>,
        current_score_total: Option<u64>,
        rollback_reason: Option<String>,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        let context_hash = context.context_hash();
        let action_id = candidate.action_id();
        let record = CandidateMemoryRecord {
            action_id: action_id.clone(),
            candidate_name: candidate.profile_name().to_owned(),
            last_tried_unix_nanos: now_unix_nanos,
            result,
            context_hash: context_hash.clone(),
            score_delta: score_delta_i64(baseline_score_total, current_score_total),
            rollback_reason: normalize_owned_reason(rollback_reason),
            cooldown_expires_unix_nanos,
        };

        if let Some(existing) = self.records.iter_mut().find(|existing| {
            existing.action_id == action_id && existing.context_hash == context_hash
        }) {
            *existing = record.clone();
        } else {
            self.records.push(record.clone());
        }

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
            .filter(|record| &record.action_id == action_id)
            .max_by_key(|record| record.last_tried_unix_nanos)
    }

    pub fn cooldown_remaining_for_action(
        &self,
        action_id: &ActionId,
        now_unix_nanos: u128,
    ) -> Option<Duration> {
        let record = self.last_for_action(action_id)?;
        let cooldown_expires = record.cooldown_expires_unix_nanos?;

        if now_unix_nanos >= cooldown_expires {
            return None;
        }

        Some(duration_from_remaining_nanos(
            cooldown_expires.saturating_sub(now_unix_nanos),
        ))
    }
}

fn target_executable_inode_from_snapshot(
    observation: &AutotuneObservation,
    snapshot: &TargetSnapshot,
) -> (Option<u64>, Option<u64>) {
    let Some(target_root_pid) = observation.target_root_pid else {
        return (None, None);
    };

    for task in snapshot.tasks.values() {
        if task.process_pid == target_root_pid || task.tid == target_root_pid {
            return (task.exe_dev, task.exe_ino);
        }
    }

    (None, None)
}

fn class_distribution_from_snapshot(snapshot: &TargetSnapshot) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for task in snapshot.tasks.values() {
        *counts.entry(task.class).or_default() += 1;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_class_distribution(
    distribution: Vec<CandidateClassCount>,
) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for item in distribution {
        if item.count == 0 {
            continue;
        }

        *counts.entry(item.class).or_default() += item.count;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_owned_reason(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn score_delta_i64(baseline_score_total: Option<u64>, current_score_total: Option<u64>) -> i64 {
    let (Some(baseline), Some(current)) = (baseline_score_total, current_score_total) else {
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

fn duration_from_remaining_nanos(remaining_nanos: u128) -> Duration {
    Duration::from_nanos(remaining_nanos.min(u64::MAX as u128) as u64)
}

fn stable_hash_hex(parts: &[String]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;

    for part in parts {
        for byte in part.len().to_string().as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= b':' as u64;
        hash = hash.wrapping_mul(0x100000001b3);

        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{ActionId, SafetyClass},
        autotune::state::SituationKind,
    };

    fn fake_candidate() -> CandidateAction {
        CandidateAction::Fake {
            action_id: ActionId("cpu-affinity-profile:test".to_owned()),
            safety_class: SafetyClass::ReversibleLowRisk,
        }
    }

    fn context_with_distribution(
        distribution: Vec<CandidateClassCount>,
    ) -> CandidateContextHashInput {
        CandidateContextHashInput {
            target_exe_dev: Some(8),
            target_exe_ino: Some(99),
            cpu_topology_signature: Some("cpu0-7:smt:on".to_owned()),
            profile_name: "fake-profile".to_owned(),
            situation: SituationKind::GameCpuSchedulerPressure,
            active_task_class_distribution: distribution,
        }
    }

    #[test]
    fn context_hash_normalizes_class_distribution_order_and_zero_counts() {
        let left = context_with_distribution(vec![
            CandidateClassCount {
                class: TaskClass::WineServer,
                count: 1,
            },
            CandidateClassCount {
                class: TaskClass::Game,
                count: 2,
            },
            CandidateClassCount {
                class: TaskClass::Game,
                count: 3,
            },
            CandidateClassCount {
                class: TaskClass::Unknown,
                count: 0,
            },
        ]);

        let right = context_with_distribution(vec![
            CandidateClassCount {
                class: TaskClass::Game,
                count: 5,
            },
            CandidateClassCount {
                class: TaskClass::WineServer,
                count: 1,
            },
        ]);

        assert_eq!(left.context_hash(), right.context_hash());
        assert_eq!(
            left.normalized().active_task_class_distribution,
            vec![
                CandidateClassCount {
                    class: TaskClass::Game,
                    count: 5,
                },
                CandidateClassCount {
                    class: TaskClass::WineServer,
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn memory_records_candidate_result_context_hash_score_delta_reason_and_cooldown() {
        let candidate = fake_candidate();
        let context = context_with_distribution(vec![CandidateClassCount {
            class: TaskClass::Game,
            count: 4,
        }]);
        let mut memory = CandidateMemory::default();

        let record = memory.record_result(
            &candidate,
            &context,
            1_000,
            CandidateMemoryResult::Reverted,
            Some(1_000),
            Some(1_125),
            Some(" candidate regressed ".to_owned()),
            Some(301_000_000_000),
        );

        assert_eq!(
            record.action_id,
            ActionId("cpu-affinity-profile:test".to_owned())
        );
        assert_eq!(record.candidate_name, "fake-profile");
        assert_eq!(record.last_tried_unix_nanos, 1_000);
        assert_eq!(record.result, CandidateMemoryResult::Reverted);
        assert_eq!(record.context_hash, context.context_hash());
        assert_eq!(record.score_delta, 125);
        assert_eq!(
            record.rollback_reason.as_deref(),
            Some("candidate regressed")
        );
        assert_eq!(record.cooldown_expires_unix_nanos, Some(301_000_000_000));
        assert_eq!(memory.records.len(), 1);
        assert_eq!(memory.latest(), Some(&record));
    }

    #[test]
    fn memory_upserts_same_action_and_context() {
        let candidate = fake_candidate();
        let context = context_with_distribution(vec![CandidateClassCount {
            class: TaskClass::Game,
            count: 4,
        }]);
        let mut memory = CandidateMemory::default();

        memory.record_attempt(&candidate, &context, 1_000, None);
        let record = memory.record_result(
            &candidate,
            &context,
            2_000,
            CandidateMemoryResult::Kept,
            Some(1_000),
            Some(800),
            None,
            Some(62_000_000_000),
        );

        assert_eq!(memory.records.len(), 1);
        assert_eq!(memory.latest(), Some(&record));
        assert_eq!(memory.records[0].result, CandidateMemoryResult::Kept);
        assert_eq!(memory.records[0].score_delta, -200);
        assert_eq!(
            memory.records[0].cooldown_expires_unix_nanos,
            Some(62_000_000_000)
        );
    }

    #[test]
    fn cooldown_remaining_uses_latest_record_for_action() {
        let candidate = fake_candidate();
        let context = context_with_distribution(vec![CandidateClassCount {
            class: TaskClass::Game,
            count: 4,
        }]);
        let action_id = candidate.action_id();
        let mut memory = CandidateMemory::default();

        memory.record_attempt(&candidate, &context, 1_000, Some(301_000_000_000));

        let remaining = memory
            .cooldown_remaining_for_action(&action_id, 300_000_000_000)
            .unwrap();

        assert_eq!(remaining.as_secs(), 1);
        assert_eq!(
            memory.cooldown_remaining_for_action(&action_id, 301_000_000_000),
            None
        );
    }
}
