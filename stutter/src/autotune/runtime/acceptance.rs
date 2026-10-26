//! Physical failure scenarios driven through the production `AutotuneRuntime`
//! decision path (runner, manager, journal, history, daemon state) against the
//! disposable owned workload. Acceptance-only; never invoked by the daemon.
use std::{fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::json;

use super::{AutotuneRuntime, AutotuneRuntimeConfig, AutotuneRuntimePhase};
use crate::{
    autotune::{
        acceptance_mutation::AcceptanceTarget,
        action_transaction::JournaledActionFailure,
        controller_journal::read_controller_journal,
        crash_acceptance::{RecoveryCheckpoint, acceptance_observation, acceptance_override},
        decision::AutotuneDecision,
        live_experiment::FaultInjectingExecutor,
    },
    daemon::policy::DaemonMode,
    daemon_policy::ActionSource,
};

/// Medium-risk mutation goes through the production privileged-service path with the explicit
/// unlock a validation harness is granted; a disposable tree owned by this user needs no root
/// worker, so the service runs in this process.
fn acceptance_daemon_config(
    target: AcceptanceTarget,
    tree_pid: u32,
) -> crate::daemon::DaemonConfig {
    let mut daemon_config = super::daemon_config_for_runtime_mode(
        target.mutation.mode(),
        ActionSource::AutotuneRuntime,
        Some(tree_pid),
        None,
    );
    let medium = target.mutation.mode() == DaemonMode::ApplyMediumRisk;
    daemon_config.autotune.unsafe_in_process_privileged_worker = medium;
    daemon_config.autotune.allow_medium_risk_apply = medium;
    if target.mutation == crate::autotune::acceptance_mutation::AcceptanceMutation::CpuPowerEpp {
        daemon_config.safety.allow_system_wide_apply = true;
        daemon_config
            .safety
            .system_wide_allowlist
            .operations
            .insert("cpu_power_epp".to_owned());
    }
    daemon_config
}

pub(crate) fn run_failure_scenario(
    out: &Path,
    tree_pid: u32,
    desired: AcceptanceTarget,
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
        acceptance_daemon_config(desired, tree_pid),
        Some(out.join("decisions.jsonl")),
    );
    config.controller_journal_path = Some(journal_path.clone());
    config.history_log = Some(out.join("history.jsonl"));
    let candidate = desired.candidate(tree_pid)?;
    config.validation_override = Some(acceptance_override(&candidate));
    let mut runtime = AutotuneRuntime::new(config);
    let observation = acceptance_observation(tree_pid);
    let decision = AutotuneDecision::StartExperiment {
        candidate,
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
        .with_context(|| format!("runner outcome type was not preserved: {error:#}"))?;
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

/// Starts a real low-risk candidate, keeps it measuring across a real system
/// suspend, and reports how the production lifecycle policy handled the resume.
pub(crate) fn run_suspend_resume(
    out: &Path,
    tree_pid: u32,
    desired: AcceptanceTarget,
    resume_timeout: std::time::Duration,
) -> anyhow::Result<()> {
    use crate::autotune::crash_acceptance::publish_acceptance_handshake;
    let journal_path = out.join("controller.json");
    let mut config = AutotuneRuntimeConfig::from_daemon_config(
        acceptance_daemon_config(desired, tree_pid),
        Some(out.join("decisions.jsonl")),
    );
    config.controller_journal_path = Some(journal_path.clone());
    config.history_log = Some(out.join("history.jsonl"));
    let candidate = desired.candidate(tree_pid)?;
    config.validation_override = Some(acceptance_override(&candidate));
    let mut runtime = AutotuneRuntime::new(config);
    let observation = acceptance_observation(tree_pid);
    runtime.apply_decision_side_effects_with_executor(
        &observation,
        &AutotuneDecision::StartExperiment {
            candidate,
            reason: "suspend/resume acceptance".into(),
        },
        "suspend/resume acceptance",
        &mut FaultInjectingExecutor::default(),
    )?;
    ensure!(
        runtime.live_experiments.has_active_experiment(),
        "no active experiment before suspend"
    );
    // A pre-suspend interval in the rolling window: it must not survive resume.
    runtime
        .controller
        .window
        .push_intervals([crate::recorder::IntervalRecord {
            samples: 10,
            ..Default::default()
        }]);
    runtime.poll_lifecycle_clock()?;
    let journal = read_controller_journal(&journal_path)?;
    publish_acceptance_handshake(
        out,
        "suspend-ready.json",
        json!({
            "journal_state": journal.state().as_str(),
            "rollback_token_present": journal.rollback_token().is_some(),
            "active_experiment": runtime.live_experiments.has_active_experiment(),
            "window_intervals": runtime.controller.window.interval_count(),
        }),
    )?;
    let deadline = std::time::Instant::now() + resume_timeout;
    let event = loop {
        if let Some(event) = runtime.poll_lifecycle_clock()? {
            break event;
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "no suspend/resume discontinuity observed"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let journal = read_controller_journal(&journal_path)?;
    let transition = runtime
        .lifecycle
        .last_transition
        .as_ref()
        .context("lifecycle transition missing")?;
    let slept_ms = match event {
        crate::daemon::lifecycle::DaemonLifecycleEvent::ResumeDetected { slept_ms } => {
            json!(slept_ms.to_string())
        }
        other => anyhow::bail!("unexpected lifecycle event {other:?}"),
    };
    let cooldown_ms = runtime
        .controller
        .state
        .cooldown_until_unix_nanos
        .map(|until| until.saturating_sub(crate::audit::unix_nanos_now()) / 1_000_000);
    publish_acceptance_handshake(
        out,
        "resume-handled.json",
        json!({
            "slept_ms": slept_ms,
            "actions": transition.actions.iter().map(|action| action.as_str()).collect::<Vec<_>>(),
            "stabilization_ms": transition.stabilization_ms,
            "window_intervals": runtime.controller.window.interval_count(),
            "active_experiment": runtime.live_experiments.has_active_experiment(),
            "journal_state": journal.state().as_str(),
            "journal_token_present": journal.rollback_token().is_some(),
            "runtime_phase": format!("{:?}", runtime.runtime_phase()),
            "cooldown_remaining_ms": cooldown_ms.map(|ms| ms.to_string()),
        }),
    )
}
