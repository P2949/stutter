use std::time::Duration;

use super::{
    key::CandidateContextHashInput,
    model::{CandidateMemory, CandidateMemoryResult, WorkloadActionMemory},
};
use crate::{actions::ActionId, autotune::planning::candidate::CandidateAction};

impl CandidateMemory {
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

    pub fn last_for_workload_action(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
    ) -> Option<&WorkloadActionMemory> {
        let action_id = candidate.action_id();
        self.workload_actions
            .iter()
            .filter(|memory| memory.action_id == action_id && memory.matches_context(context))
            .max_by_key(|memory| memory.last_seen_unix_nanos)
    }

    pub fn cooldown_remaining_for_workload_action(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
    ) -> Option<Duration> {
        let memory = self.last_for_workload_action(candidate, context)?;
        let cooldown_until = memory.cooldown_until_unix_nanos?;
        if now_unix_nanos >= cooldown_until {
            return None;
        }

        Some(duration_from_remaining_nanos(
            cooldown_until.saturating_sub(now_unix_nanos),
        ))
    }

    pub fn workload_rank_hint(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
    ) -> Option<i32> {
        let memory = self.last_for_workload_action(candidate, context)?;
        match memory.last_result {
            CandidateMemoryResult::Kept => Some(-100),
            CandidateMemoryResult::Reverted
            | CandidateMemoryResult::Faulted
            | CandidateMemoryResult::Rejected => Some(100),
            CandidateMemoryResult::Inconclusive => Some(20),
            CandidateMemoryResult::Tried => Some(0),
        }
    }
}

fn duration_from_remaining_nanos(remaining_nanos: u128) -> Duration {
    Duration::from_nanos(remaining_nanos.min(u64::MAX as u128) as u64)
}
