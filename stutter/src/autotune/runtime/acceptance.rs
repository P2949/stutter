//! Physical failure scenarios driven through the production `AutotuneRuntime`
//! decision path (runner, manager, journal, history, daemon state) against the
//! disposable owned workload. Acceptance-only; never invoked by the daemon.
use std::{fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::json;

use super::{AutotuneRuntime, AutotuneRuntimeConfig, AutotuneRuntimePhase};
use crate::{
    autotune::{
        action_transaction::JournaledActionFailure,
        controller_journal::read_controller_journal,
        crash_acceptance::{RecoveryCheckpoint, acceptance_candidate, acceptance_observation},
        decision::AutotuneDecision,
        live_experiment::FaultInjectingExecutor,
    },
    daemon::policy::DaemonMode,
    daemon_policy::ActionSource,
};

pub(crate) fn run_failure_scenario(
    out: &Path,
    tree_pid: u32,
    desired: crate::affinity::CpuMask,
    scenario: RecoveryCheckpoint,
) -> anyhow::Result<()> {
    let verify_failure = match scenario {
        RecoveryCheckpoint::PartialApplyFailure => None,
        RecoveryCheckpoint::VerifyFailure => Some(false),
        RecoveryCheckpoint::RollbackFailure => Some(true),
        other => anyhow::bail!("{} is not a failure scenario", other.as_str()),
    };
    let journal_path = out.join("controller.json");
    let mut config = AutotuneRuntimeConfig::from_daemon_config(
        super::daemon_config_for_runtime_mode(
            DaemonMode::ApplyLowRisk,
            ActionSource::AutotuneRuntime,
            Some(tree_pid),
            None,
        ),
        Some(out.join("decisions.jsonl")),
    );
    config.controller_journal_path = Some(journal_path.clone());
    config.history_log = Some(out.join("history.jsonl"));
    let mut runtime = AutotuneRuntime::new(config);
    let observation = acceptance_observation(tree_pid);
    let decision = AutotuneDecision::StartExperiment {
        candidate: acceptance_candidate(tree_pid, desired)?,
        reason: "physical failure acceptance".into(),
    };
    let mut executor = FaultInjectingExecutor {
        verify_failure,
        ..Default::default()
    };
    let error = runtime
        .apply_decision_side_effects_with_executor(
            &observation,
            &decision,
            "physical failure acceptance",
            &mut executor,
        )
        .err()
        .context("injected failure unexpectedly succeeded")?;
    let typed = error
        .downcast_ref::<JournaledActionFailure>()
        .context("runner outcome type was not preserved")?;
    let journal = read_controller_journal(&journal_path)?;
    let phase = runtime.runtime_phase();
    ensure!(
        phase == AutotuneRuntimePhase::Faulted,
        "runtime not faulted"
    );
    // A faulted runtime must refuse more mutation and never reach the executor.
    let retry = runtime.apply_decision_side_effects_with_executor(
        &observation,
        &decision,
        "must stay blocked",
        &mut executor,
    );
    let report = json!({
        "scenario": scenario.as_str(),
        "error": format!("{error:#}"),
        "rollback_present": typed.rollback.is_some(),
        "rollback_completed": typed.rollback_completed,
        "runtime_phase": format!("{phase:?}"),
        "journal_state": journal.state().as_str(),
        "journal_token_present": journal.rollback_token().is_some(),
        "future_mutation_rejected": retry.is_err(),
        "executor_apply_calls": executor.apply_calls,
    });
    let temporary = out.join("failure-report.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&report)?)?;
    fs::rename(temporary, out.join("failure-report.json"))?;
    Ok(())
}
