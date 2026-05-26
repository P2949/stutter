use std::collections::{BTreeMap, BTreeSet};

use super::model::CandidateMemory;

pub(super) fn mark_loaded_identity_collisions(memory: &mut CandidateMemory) {
    let colliding_context_hashes = colliding_record_context_hashes(memory);
    for record in &mut memory.records {
        if colliding_context_hashes.contains(record.context_hash.as_str()) {
            record.degraded_reason = Some(format!(
                "candidate memory identity collision for context_hash {}",
                record.context_hash.as_str()
            ));
        }
    }

    let colliding_workload_hashes = colliding_workload_hashes(memory);
    for memory in &mut memory.workload_actions {
        if colliding_workload_hashes.contains(memory.workload_hash.as_str()) {
            memory.degraded_reason = Some(format!(
                "candidate memory identity collision for workload_hash {}",
                memory.workload_hash
            ));
        }
    }
}

fn colliding_record_context_hashes(memory: &CandidateMemory) -> BTreeSet<String> {
    let mut seen = BTreeMap::<String, String>::new();
    let mut collisions = BTreeSet::new();

    for record in &memory.records {
        let Some(summary) = record.identity_summary.as_ref() else {
            continue;
        };
        let key = record.context_hash.as_str().to_owned();
        let summary = summary.as_str().to_owned();

        if let Some(previous) = seen.get(&key) {
            if previous != &summary {
                collisions.insert(key);
            }
        } else {
            seen.insert(key, summary);
        }
    }

    collisions
}

fn colliding_workload_hashes(memory: &CandidateMemory) -> BTreeSet<String> {
    let mut seen = BTreeMap::<String, String>::new();
    let mut collisions = BTreeSet::new();

    for memory in &memory.workload_actions {
        let Some(summary) = memory.identity_summary.as_ref() else {
            continue;
        };
        let key = memory.workload_hash.clone();
        let summary = summary.as_str().to_owned();

        if let Some(previous) = seen.get(&key) {
            if previous != &summary {
                collisions.insert(key);
            }
        } else {
            seen.insert(key, summary);
        }
    }

    collisions
}
