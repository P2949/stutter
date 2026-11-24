//! The raw record of an owned-pressure control, and the receipt rebuilt from it. The receipt used to be a set of claims
//! (`load_owned_by_campaign`, `action_applied`, `restored_independently`) a producer wrote; here none of them is taken on
//! trust. The control leaves its raw account: the load's own lifecycle record, the runnable-latency population counted
//! before the load and the one counted under it (the controller's canonical histogram), the gate's receipt for the action
//! the controller applied under that load and the privileged worker's own export of that transaction. `reconstruct`
//! derives every field of the receipt from them: the load ran and was joined; the populations are valid, one strictly before
//! the load and one inside it; the percentiles are the production objective's own; the action was held inside the loaded
//! window and the worker shows it prepared, applied and restored on that very process; and the kernel state after the
//! restoration is the state before the action.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{LoadRecord, PressureControlReceipt};
use crate::{
    autotune::{
        objective::ObjectiveSignals,
        research::{ResearchAction, child_verify::AttemptProcess, worker_provenance},
    },
    metrics::LatencyHistogramBucket,
};

/// The file, beside the control's receipt, holding the raw record.
pub const EVIDENCE_FILE: &str = "pressure-control-evidence.json";

/// A runnable-delay population counted over `[start_unix, end_unix)` in the canonical layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    pub start_unix: u64,
    pub end_unix: u64,
    pub histogram: Vec<LatencyHistogramBucket>,
    pub max_ns: u64,
}

/// What the controller did under the load: the gate's receipt of it and the worker's own export of the transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRecord {
    pub receipt: Value,
    pub worker_transactions: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PressureControlEvidence {
    pub schema: u32,
    pub binary_sha256: String,
    pub captured_unix: u64,
    pub load: LoadRecord,
    pub baseline: Population,
    pub loaded: Population,
    pub action: ActionRecord,
}

/// The population's event count and its 95th percentile in milliseconds, from the production objective's own histogram
/// reading, or why it is not a valid population.
fn read(population: &Population) -> Result<(u64, f64), &'static str> {
    let mut signals = ObjectiveSignals::default();
    signals.set_runnable_latency_from_histogram(&population.histogram, population.max_ns);
    let events = population
        .histogram
        .iter()
        .try_fold(0u64, |total, bucket| total.checked_add(bucket.count))
        .ok_or("PressurePopulationInvalid")?;
    let p95 = signals
        .runnable_latency_p95_ns
        .filter(|_| population.start_unix < population.end_unix)
        .ok_or("PressurePopulationInvalid")?;
    Ok((events, p95 as f64 / 1_000_000.0))
}

fn action_of(receipt: &Value) -> Option<ResearchAction> {
    match receipt["action"].as_str()? {
        "CpuAffinityProfile" => Some(ResearchAction::CpuAffinityProfile),
        "CgroupPlacement" => Some(ResearchAction::CgroupPlacement),
        _ => None,
    }
}

/// The receipt this evidence supports, derived and never asserted.
pub fn reconstruct(
    evidence: &PressureControlEvidence,
) -> Result<PressureControlReceipt, &'static str> {
    if evidence.schema != 1 {
        return Err("PressureControlSchemaUnsupported");
    }
    let load = &evidence.load;
    // The campaign started the load and it ended: a record that never ended, or whose workers were not all joined, is not
    // a load the campaign owned.
    let owned = load.workers > 0
        && load.started_unix < load.stopped_unix
        && load.all_joined
        && load.pid.as_u32() != 0
        && load.start_ticks != 0;
    // One population before the load, one inside it, each valid.
    let (baseline, loaded) = (&evidence.baseline, &evidence.loaded);
    if baseline.end_unix > load.started_unix
        || loaded.start_unix < load.started_unix
        || loaded.end_unix > load.stopped_unix
    {
        return Err("PressureWindowsNotOrderedAroundTheLoad");
    }
    let (baseline_events, baseline_p95) = read(baseline)?;
    let (loaded_events, loaded_p95) = read(loaded)?;
    // The action the controller applied: held inside the loaded window, and the worker shows that very transaction on that
    // very process prepared, applied and restored.
    let receipt = &evidence.action.receipt;
    let held = match (
        receipt["applied_unix"].as_u64(),
        receipt["settled_unix"].as_u64(),
    ) {
        (Some(applied), Some(settled)) => {
            applied < settled && applied >= loaded.start_unix && settled <= loaded.end_unix
        }
        _ => false,
    };
    let worker_shows_it = (|| {
        let process = AttemptProcess {
            slot: 0,
            pid: stutter_core::ids::Pid::new(u32::try_from(receipt["pid"].as_u64()?).ok()?),
            start_ticks: receipt["start_ticks"].as_u64()?,
            uid: u32::try_from(receipt["uid"].as_u64()?).ok()?,
            started_unix: loaded.start_unix,
            completed_unix: loaded.end_unix,
        };
        worker_provenance::verify(
            &serde_json::to_vec(&evidence.action.worker_transactions).ok()?,
            action_of(receipt)?,
            &Value::Array(vec![receipt.clone()]),
            &[process],
        )
        .ok()
    })()
    .is_some();
    // The kernel state after the restoration is the state before the action, as the gate read both.
    let state_restored =
        !receipt["original"].is_null() && receipt["original"] == receipt["restored"];
    Ok(PressureControlReceipt {
        schema: 1,
        binary_sha256: evidence.binary_sha256.clone(),
        captured_unix: evidence.captured_unix,
        load_owned_by_campaign: owned,
        scoped_events: baseline_events.saturating_add(loaded_events),
        baseline_runnable_p95_ms: baseline_p95,
        loaded_runnable_p95_ms: loaded_p95,
        action_applied: held && worker_shows_it,
        restored_independently: held && worker_shows_it && state_restored,
    })
}

#[cfg(test)]
#[path = "pressure_evidence_tests.rs"]
mod tests;
