use std::{fs, path::Path, thread, time::Duration};

use serde::Serialize;

use crate::{
    actions::{ActionId, SafetyClass},
    affinity,
    autotune::emergency_restore::{
        AutotuneRestoreCommandInput, AutotuneRestoreOutcome, AutotuneRestoreStatus,
        restore_known_autotune_actions,
    },
    commands::{input, restore},
    config_file::{self, UserConfigFile},
    daemon::{
        ActionDescriptor, ActionEffectScope, ActionSource, CapabilityProbe, DaemonAcceptanceReport,
        DaemonCapabilities, DaemonConfig, DaemonOverheadMonitor, DaemonOverheadReport, DaemonPhase,
        DaemonPolicy, DaemonPolicyBuildInput, DaemonPolicyExplanation, DaemonPreset,
        DaemonProfileEnvironment, DaemonProfileValidation, DaemonSoakReport, DaemonState,
        DaemonStateSnapshotWriter, DaemonStateStore, DaemonStatusExplanation, DaemonWatchdogConfig,
        DaemonWatchdogInputs, DaemonWatchdogReport, DaemonWorkloadProfile, PolicyExplanation,
        PolicyIntent, RollbackRequirement, SystemHealthMonitor, SystemHealthProbeRoot,
        SystemHealthSnapshot, build_daemon_policy, default_daemon_state_snapshot_path,
        evaluate_daemon_watchdog, load_daemon_state, policy_context_from_daemon_status,
        run_fake_daemon_acceptance_suite, run_fake_daemon_soak,
    },
    profile_restore,
    remote::AgentAutotuneLimits,
};

#[derive(Clone, Debug, Serialize)]
struct DaemonConfigExplainOutput {
    config: DaemonConfig,
    policy: DaemonPolicy,
    explanation: PolicyExplanation,
    agent_autotune_limits: AgentAutotuneLimits,
    user_config_loaded: bool,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonPolicyExplainOutput {
    config: DaemonConfig,
    policy: DaemonPolicy,
    explanation: DaemonPolicyExplanation,
    user_config_loaded: bool,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonProfilesListOutput {
    state_path: String,
    state_loaded: bool,
    profiles: Vec<DaemonWorkloadProfile>,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonProfilesForgetOutput {
    state_path: String,
    dry_run: bool,
    before_count: usize,
    removed_count: usize,
    remaining_count: usize,
    removed: Vec<DaemonWorkloadProfile>,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonProfilesExplainOutput {
    state_path: String,
    state_loaded: bool,
    current_environment: DaemonProfileEnvironment,
    profiles: Vec<DaemonProfileExplanationOutput>,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonProfileExplanationOutput {
    profile: DaemonWorkloadProfile,
    validation: DaemonProfileValidation,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonExplainOutput {
    status: DaemonStatusOutput,
    policy: DaemonPolicy,
    policy_explanation: DaemonPolicyExplanation,
    status_explanation: DaemonStatusExplanation,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonWhyNotOptimizeOutput {
    state_path: String,
    state_loaded: bool,
    mode: crate::daemon::DaemonMode,
    phase: DaemonPhase,
    health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    why_no_optimize: Vec<String>,
    recent_decisions: Vec<DaemonRecentDecision>,
    manual_restore_command: String,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonWhatChangedOutput {
    state_path: String,
    state_loaded: bool,
    mode: crate::daemon::DaemonMode,
    phase: DaemonPhase,
    health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    what_changed: Vec<String>,
    recent_decisions: Vec<DaemonRecentDecision>,
    manual_restore_command: String,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonStatusOutput {
    state_path: String,
    state_loaded: bool,
    state: DaemonState,
    capabilities: DaemonCapabilities,
    current_health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    manual_restore_command: String,
    recent_decisions: Vec<DaemonRecentDecision>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct DaemonRecentDecision {
    unix_nanos: u128,
    phase: String,
    mode: String,
    decision: String,
    candidate_name: Option<String>,
    action_id: Option<String>,
    rollback_performed: bool,
    reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DaemonWatchSignature {
    phase: DaemonPhase,
    active_action_id: Option<String>,
    rollback_action_id: Option<String>,
    rollback_available: bool,
    fault_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonDoctorReport {
    state_path: String,
    state_load_ok: bool,
    state_uncertain: bool,
    safe_observe_only_required: bool,
    manual_restore_command: String,
    checks: Vec<DaemonDoctorCheck>,
    capabilities: DaemonCapabilities,
    current_health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonDoctorCheck {
    name: String,
    passed: bool,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct DaemonResetStateReport {
    state_path: String,
    dry_run: bool,
    state_exists: bool,
    backup_path: Option<String>,
    reset_state: DaemonState,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug)]
struct DaemonRestoreCommandOutcome {
    autotune: AutotuneRestoreOutcome,
    profile: restore::ProfileRestoreCommandOutcome,
}

pub fn run_config_explain_command(
    input: input::DaemonConfigExplainCommandInput,
) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    let output = build_config_explain_output_from_user_config(
        user_config.as_ref(),
        input.preset.as_deref(),
    )?;

    if input.json {
        println!("{}", render_config_explain_json(&output)?);
    } else {
        print!("{}", render_config_explain_text(&output));
    }

    Ok(())
}

pub fn run_policy_explain_command(
    input: input::DaemonPolicyExplainCommandInput,
) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    let output = build_policy_explain_output_from_user_config(
        user_config.as_ref(),
        input.preset.as_deref(),
    )?;

    if input.json {
        println!("{}", render_policy_explain_json(&output)?);
    } else {
        print!("{}", render_policy_explain_text(&output));
    }

    Ok(())
}

pub fn run_privileged_worker_command(
    input: input::PrivilegedWorkerCommandInput,
) -> anyhow::Result<()> {
    crate::daemon::privilege::run_privileged_worker(&input.socket)
}

pub fn run_profiles_command(input: input::DaemonProfilesCommandInput) -> anyhow::Result<()> {
    match input {
        input::DaemonProfilesCommandInput::List(input) => run_profiles_list_command(input),
        input::DaemonProfilesCommandInput::Forget(input) => run_profiles_forget_command(input),
        input::DaemonProfilesCommandInput::Explain(input) => run_profiles_explain_command(input),
    }
}

fn run_profiles_list_command(input: input::DaemonProfilesListCommandInput) -> anyhow::Result<()> {
    let output = build_profiles_list_output();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_list_text(&output));
    }

    Ok(())
}

fn run_profiles_forget_command(
    input: input::DaemonProfilesForgetCommandInput,
) -> anyhow::Result<()> {
    let output = forget_daemon_profiles(&input)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_forget_text(&output));
    }

    Ok(())
}

fn run_profiles_explain_command(
    input: input::DaemonProfilesExplainCommandInput,
) -> anyhow::Result<()> {
    let output = build_profiles_explain_output(input.workload_identity_hash.as_deref());

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_profiles_explain_text(&output));
    }

    Ok(())
}

pub fn run_explain_command(input: input::DaemonExplainCommandInput) -> anyhow::Result<()> {
    let output = build_explain_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_explain_text(&output));
    }

    Ok(())
}

pub fn run_why_not_optimize_command(
    input: input::DaemonWhyNotOptimizeCommandInput,
) -> anyhow::Result<()> {
    let output = build_why_not_optimize_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_why_not_optimize_text(&output));
    }

    Ok(())
}

pub fn run_what_changed_command(input: input::DaemonWhatChangedCommandInput) -> anyhow::Result<()> {
    let output = build_what_changed_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_what_changed_text(&output));
    }

    Ok(())
}

pub fn run_status_command(input: input::DaemonStatusCommandInput) -> anyhow::Result<()> {
    let output = build_status_output_with_recent_decisions(input.explain_last);

    if input.json {
        println!("{}", render_status_json(&output)?);
    } else {
        print!("{}", render_status_text(&output));
    }

    Ok(())
}

pub fn run_watch_command(input: input::DaemonWatchCommandInput) -> anyhow::Result<()> {
    let iterations = input.iterations.unwrap_or(u64::MAX);
    let mut previous = None;

    for index in 0..iterations {
        let output = build_status_output_with_recent_decisions(input.explain_last);
        let signature = DaemonWatchSignature::from_output(&output);

        if index == 0 || input.verbose {
            print!("{}", render_watch_line(&output));
        }
        if let Some(notification) = previous
            .as_ref()
            .and_then(|old| render_watch_notification(old, &signature))
        {
            println!("notification: {notification}");
        }
        if input.verbose {
            print!("{}", render_status_text(&output));
        }

        previous = Some(signature);
        if index + 1 < iterations {
            thread::sleep(Duration::from_millis(input.interval_ms));
        }
    }

    Ok(())
}

pub fn run_doctor_command(input: input::DaemonDoctorCommandInput) -> anyhow::Result<()> {
    let report = build_daemon_doctor_report();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_daemon_doctor_text(&report));
    }

    Ok(())
}

pub fn run_reset_state_command(input: input::DaemonResetStateCommandInput) -> anyhow::Result<()> {
    let report = reset_daemon_state(input.dry_run)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_reset_state_text(&report));
    }

    Ok(())
}

pub fn run_bench_overhead_command(
    input: input::DaemonBenchOverheadCommandInput,
) -> anyhow::Result<()> {
    let report = DaemonOverheadMonitor::default()
        .sample_over_duration(Duration::from_millis(input.duration_ms));

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_bench_overhead_text(&report));
    }

    Ok(())
}

pub fn run_soak_command(input: input::DaemonSoakCommandInput) -> anyhow::Result<()> {
    let report = run_fake_daemon_soak(&input.config);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_soak_text(&report));
    }

    if !report.passed {
        anyhow::bail!("daemon fake soak exceeded one or more budgets");
    }

    Ok(())
}

pub fn run_acceptance_command(input: input::DaemonAcceptanceCommandInput) -> anyhow::Result<()> {
    let report = run_fake_daemon_acceptance_suite();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_acceptance_text(&report));
    }

    if !report.passed {
        anyhow::bail!("daemon acceptance suite failed one or more steps");
    }

    Ok(())
}

pub fn run_pause_command(_: input::DaemonPauseCommandInput) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let mut store = daemon_state_store_for_path(&state_path)?;

    store.pause("operator requested daemon pause")?;

    println!(
        "daemon paused; state_path={} manual_restore_command=\"stutter daemon emergency-restore\"",
        state_path.display()
    );
    Ok(())
}

pub fn run_resume_command(_: input::DaemonResumeCommandInput) -> anyhow::Result<()> {
    let state_path = default_daemon_state_snapshot_path();
    let mut store = daemon_state_store_for_path(&state_path)?;

    store.resume("operator requested daemon resume")?;

    println!("daemon resumed; state_path={}", state_path.display());
    Ok(())
}

pub fn run_restore_command(input: input::DaemonRestoreCommandInput) -> anyhow::Result<()> {
    run_restore_command_with_profile_paths(
        input,
        None,
        None,
        None,
        affinity::default_restore_path(),
        profile_restore::default_restore_path(),
    )?;
    Ok(())
}

fn run_restore_command_with_profile_paths(
    input: input::DaemonRestoreCommandInput,
    journal_path: Option<std::path::PathBuf>,
    audit_path: Option<std::path::PathBuf>,
    history_path: Option<std::path::PathBuf>,
    affinity_path: std::path::PathBuf,
    profile_path: std::path::PathBuf,
) -> anyhow::Result<DaemonRestoreCommandOutcome> {
    let outcome = restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path,
        audit_path,
        history_path,
        dry_run: input.dry_run,
    })?;

    for message in &outcome.messages {
        println!("{message}");
    }

    let profile_outcome =
        restore::restore_profile_state_from_paths(affinity_path, profile_path, input.dry_run)?;
    for message in &profile_outcome.messages {
        println!("{message}");
    }
    let restore_summary = daemon_restore_summary_fields(&outcome, &profile_outcome);
    println!("daemon restore summary: {restore_summary}");

    if input.dry_run {
        return Ok(DaemonRestoreCommandOutcome {
            autotune: outcome,
            profile: profile_outcome,
        });
    }

    let state_path = default_daemon_state_snapshot_path();
    let mut store = daemon_state_store_for_path(&state_path)?;
    let command = if input.emergency {
        "daemon_emergency_restore"
    } else {
        "daemon_restore"
    };

    match outcome.status {
        AutotuneRestoreStatus::Clean | AutotuneRestoreStatus::Restored => {
            store.mark_restored(format!("{command} completed {restore_summary}"))?;
            println!(
                "daemon restore state updated; state_path={}",
                state_path.display()
            );
            Ok(DaemonRestoreCommandOutcome {
                autotune: outcome,
                profile: profile_outcome,
            })
        }
        AutotuneRestoreStatus::ApplyingWithoutRollbackToken | AutotuneRestoreStatus::Faulted => {
            store.mark_fault(
                store.state().mode,
                format!("{command} could not complete {restore_summary}"),
                Some("stutter daemon emergency-restore --dry-run".to_owned()),
            )?;
            anyhow::bail!(
                "daemon restore did not complete safely; status={:?}",
                outcome.status
            );
        }
        AutotuneRestoreStatus::DryRun => Ok(DaemonRestoreCommandOutcome {
            autotune: outcome,
            profile: profile_outcome,
        }),
    }
}

fn daemon_restore_summary_fields(
    outcome: &AutotuneRestoreOutcome,
    profile_outcome: &restore::ProfileRestoreCommandOutcome,
) -> String {
    restore::RestoreSummaryFields::from_profile(
        format!("{:?}", outcome.status),
        outcome.restored_actions,
        outcome.failed_actions,
        outcome.skipped_actions,
        profile_outcome,
    )
    .render_fields()
}

fn build_config_explain_output_from_user_config(
    user_config: Option<&UserConfigFile>,
    preset: Option<&str>,
) -> anyhow::Result<DaemonConfigExplainOutput> {
    let config =
        config_file::daemon_config_from_user_config(user_config, preset, ActionSource::Cli)?;
    let agent_autotune_limits = config_file::agent_autotune_limits_from_user_config(user_config)?;
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });
    let descriptor = daemon_config_explain_descriptor();
    let explanation = policy.explain_action(PolicyIntent::Observe, &descriptor);

    Ok(DaemonConfigExplainOutput {
        config,
        policy,
        explanation,
        agent_autotune_limits,
        user_config_loaded: user_config.is_some(),
    })
}

fn build_policy_explain_output_from_user_config(
    user_config: Option<&UserConfigFile>,
    preset: Option<&str>,
) -> anyhow::Result<DaemonPolicyExplainOutput> {
    let config_output = build_config_explain_output_from_user_config(user_config, preset)?;
    let explanation = DaemonPolicyExplanation::from_policy(&config_output.policy);

    Ok(DaemonPolicyExplainOutput {
        config: config_output.config,
        policy: config_output.policy,
        explanation,
        user_config_loaded: config_output.user_config_loaded,
    })
}

#[cfg(test)]
fn build_status_output() -> DaemonStatusOutput {
    build_status_output_with_recent_decisions(10)
}

fn configured_system_health_snapshot() -> SystemHealthSnapshot {
    system_health_snapshot_from_user_config_result(
        config_file::load_user_config(),
        SystemHealthProbeRoot::default(),
    )
}

fn system_health_snapshot_from_user_config_result(
    user_config: anyhow::Result<Option<UserConfigFile>>,
    root: SystemHealthProbeRoot,
) -> SystemHealthSnapshot {
    match user_config {
        Ok(user_config) => {
            match system_health_monitor_from_user_config_with_root(
                user_config.as_ref(),
                root.clone(),
            ) {
                Ok(monitor) => monitor.probe(),
                Err(err) => system_health_snapshot_with_config_error(root, err),
            }
        }
        Err(err) => system_health_snapshot_with_config_error(root, err),
    }
}

fn system_health_snapshot_with_config_error(
    root: SystemHealthProbeRoot,
    err: anyhow::Error,
) -> SystemHealthSnapshot {
    log::warn!("daemon_health_config_load_failed err={err:#}; blocking apply");

    let monitor = SystemHealthMonitor::new(root, Default::default());
    let mut inputs = monitor.probe_inputs();
    inputs
        .probe_errors
        .push(format!("daemon_config_load_failed: {err:#}"));
    monitor.evaluate(inputs)
}

#[cfg(test)]
fn system_health_monitor_from_user_config(
    user_config: Option<&UserConfigFile>,
) -> anyhow::Result<SystemHealthMonitor> {
    system_health_monitor_from_user_config_with_root(user_config, SystemHealthProbeRoot::default())
}

fn system_health_monitor_from_user_config_with_root(
    user_config: Option<&UserConfigFile>,
    root: SystemHealthProbeRoot,
) -> anyhow::Result<SystemHealthMonitor> {
    let thresholds = config_file::daemon_health_thresholds_from_user_config(
        user_config,
        None,
        ActionSource::Cli,
    )?;
    Ok(SystemHealthMonitor::new(root, thresholds))
}

fn build_status_output_with_recent_decisions(recent_decision_limit: usize) -> DaemonStatusOutput {
    let state_path = default_daemon_state_snapshot_path();
    let (state_loaded, state) = match load_daemon_state(&state_path) {
        Ok(state) => (true, state),
        Err(err) => {
            log::debug!(
                "daemon_status_state_load_failed path={} err={err:#}",
                state_path.display()
            );
            (false, DaemonState::default())
        }
    };

    let current_health = configured_system_health_snapshot();
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&state, &current_health),
        &DaemonWatchdogConfig::default(),
    );

    DaemonStatusOutput {
        state_path: state_path.display().to_string(),
        state_loaded,
        state,
        capabilities: CapabilityProbe::default().probe(),
        current_health,
        watchdog,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        recent_decisions: load_recent_daemon_decisions(recent_decision_limit),
    }
}

fn load_daemon_state_for_profile_commands() -> (std::path::PathBuf, bool, DaemonState) {
    let state_path = default_daemon_state_snapshot_path();
    match load_daemon_state(&state_path) {
        Ok(state) => (state_path, true, state),
        Err(err) => {
            log::debug!(
                "daemon_profile_state_load_failed path={} err={err:#}",
                state_path.display()
            );
            (state_path, false, DaemonState::default())
        }
    }
}

fn build_profiles_list_output() -> DaemonProfilesListOutput {
    let (state_path, state_loaded, state) = load_daemon_state_for_profile_commands();
    build_profiles_list_output_from_state(state_path.display().to_string(), state_loaded, &state)
}

fn build_profiles_list_output_from_state(
    state_path: String,
    state_loaded: bool,
    state: &DaemonState,
) -> DaemonProfilesListOutput {
    DaemonProfilesListOutput {
        state_path,
        state_loaded,
        profiles: state.profile_memory.sorted_profiles(),
    }
}

fn build_profiles_explain_output(
    workload_identity_hash: Option<&str>,
) -> DaemonProfilesExplainOutput {
    let (state_path, state_loaded, state) = load_daemon_state_for_profile_commands();
    build_profiles_explain_output_from_state(
        state_path.display().to_string(),
        state_loaded,
        &state,
        workload_identity_hash,
        DaemonProfileEnvironment::current(),
        crate::audit::unix_nanos_now(),
    )
}

fn build_profiles_explain_output_from_state(
    state_path: String,
    state_loaded: bool,
    state: &DaemonState,
    workload_identity_hash: Option<&str>,
    current_environment: DaemonProfileEnvironment,
    now_unix_nanos: u128,
) -> DaemonProfilesExplainOutput {
    let profiles = state
        .profile_memory
        .sorted_profiles()
        .into_iter()
        .filter(|profile| {
            workload_identity_hash
                .map(|hash| profile.workload_identity_hash == hash)
                .unwrap_or(true)
        })
        .map(|profile| {
            let validation = profile.validation(&current_environment, now_unix_nanos);
            DaemonProfileExplanationOutput {
                profile,
                validation,
            }
        })
        .collect();

    DaemonProfilesExplainOutput {
        state_path,
        state_loaded,
        current_environment,
        profiles,
    }
}

fn forget_daemon_profiles(
    input: &input::DaemonProfilesForgetCommandInput,
) -> anyhow::Result<DaemonProfilesForgetOutput> {
    let state_path = default_daemon_state_snapshot_path();
    let mut state = if state_path.exists() {
        load_daemon_state(&state_path)?
    } else {
        DaemonState::default()
    };

    let before_count = state.profile_memory.profiles.len();
    let removed = state.profile_memory.forget_matching(
        input.workload_identity_hash.as_deref(),
        input.candidate.as_deref(),
        input.all,
    );
    let remaining_count = state.profile_memory.profiles.len();

    if !input.dry_run {
        DaemonStateSnapshotWriter::new(&state_path).write(&state)?;
    }

    Ok(DaemonProfilesForgetOutput {
        state_path: state_path.display().to_string(),
        dry_run: input.dry_run,
        before_count,
        removed_count: removed.len(),
        remaining_count,
        removed,
    })
}

fn build_explain_output(recent_decision_limit: usize) -> DaemonExplainOutput {
    let status = build_status_output_with_recent_decisions(recent_decision_limit);
    let policy = build_policy_from_daemon_status(&status);
    let policy_context = policy_context_from_daemon_status(
        &status.state,
        &status.current_health,
        &status.capabilities,
    );
    let policy_explanation =
        DaemonPolicyExplanation::from_policy_with_context(&policy, &policy_context);
    let status_explanation = DaemonStatusExplanation::from_state_health_watchdog(
        &status.state,
        &status.current_health,
        &status.watchdog,
    );

    DaemonExplainOutput {
        status,
        policy,
        policy_explanation,
        status_explanation,
    }
}

fn build_why_not_optimize_output(recent_decision_limit: usize) -> DaemonWhyNotOptimizeOutput {
    let explain = build_explain_output(recent_decision_limit);
    why_not_optimize_output_from_explain(explain)
}

fn why_not_optimize_output_from_explain(
    explain: DaemonExplainOutput,
) -> DaemonWhyNotOptimizeOutput {
    DaemonWhyNotOptimizeOutput {
        state_path: explain.status.state_path.clone(),
        state_loaded: explain.status.state_loaded,
        mode: explain.status.state.mode,
        phase: explain.status.state.phase,
        health: explain.status.current_health.clone(),
        watchdog: explain.status.watchdog.clone(),
        why_no_optimize: explain.status_explanation.why_no_optimize.clone(),
        recent_decisions: explain.status.recent_decisions.clone(),
        manual_restore_command: explain.status.manual_restore_command.clone(),
    }
}

fn build_what_changed_output(recent_decision_limit: usize) -> DaemonWhatChangedOutput {
    let explain = build_explain_output(recent_decision_limit);
    what_changed_output_from_explain(explain)
}

fn what_changed_output_from_explain(explain: DaemonExplainOutput) -> DaemonWhatChangedOutput {
    DaemonWhatChangedOutput {
        state_path: explain.status.state_path.clone(),
        state_loaded: explain.status.state_loaded,
        mode: explain.status.state.mode,
        phase: explain.status.state.phase,
        health: explain.status.current_health.clone(),
        watchdog: explain.status.watchdog.clone(),
        what_changed: explain.status_explanation.what_changed.clone(),
        recent_decisions: explain.status.recent_decisions.clone(),
        manual_restore_command: explain.status.manual_restore_command.clone(),
    }
}

fn build_policy_from_daemon_status(status: &DaemonStatusOutput) -> DaemonPolicy {
    build_policy_from_daemon_state_with_user_config_result(
        &status.state,
        status.state_loaded,
        config_file::load_user_config(),
    )
}

fn build_policy_from_daemon_state_with_user_config_result(
    state: &DaemonState,
    state_loaded: bool,
    user_config: anyhow::Result<Option<UserConfigFile>>,
) -> DaemonPolicy {
    let config = match user_config {
        Ok(user_config) => {
            build_daemon_config_from_state(state, state_loaded, user_config.as_ref())
                .unwrap_or_else(|err| {
                    log::warn!("daemon_policy_config_build_failed err={err:#}; using observe-only");
                    DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
                })
        }
        Err(err) => {
            log::warn!("daemon_policy_config_load_failed err={err:#}; using observe-only");
            DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
        }
    };

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

#[cfg(test)]
fn build_policy_from_daemon_state(state: &DaemonState) -> DaemonPolicy {
    build_policy_from_daemon_state_with_user_config_result(state, true, Ok(None))
}

fn build_daemon_config_from_state(
    state: &DaemonState,
    state_loaded: bool,
    user_config: Option<&UserConfigFile>,
) -> anyhow::Result<DaemonConfig> {
    let mut config =
        config_file::daemon_config_from_user_config(user_config, None, ActionSource::Cli)?;

    if state_loaded {
        config.mode = state.mode;
    }

    apply_daemon_state_target_to_config(&mut config, state);
    Ok(config)
}

fn apply_daemon_state_target_to_config(config: &mut DaemonConfig, state: &DaemonState) {
    config.target.require_explicit_target = config.mode.supports_apply();
    if let Some(target) = state.active_target.as_ref() {
        if let Some(root_pid) = target.root_pid
            && !config.target.tree_pids.contains(&root_pid)
        {
            config.target.tree_pids.push(root_pid);
        }
        config.target.watch_process = target.comm.clone();
    }
}

fn load_recent_daemon_decisions(limit: usize) -> Vec<DaemonRecentDecision> {
    if limit == 0 {
        return Vec::new();
    }

    let path = crate::autotune::history::default_autotune_history_path();
    let Ok(events) = crate::autotune::history::read_autotune_history_events(&path) else {
        return Vec::new();
    };

    events
        .into_iter()
        .rev()
        .take(limit)
        .map(|event| DaemonRecentDecision {
            unix_nanos: event.unix_nanos,
            phase: format!("{:?}", event.phase),
            mode: format!("{:?}", event.mode),
            decision: event.decision.decision,
            candidate_name: event.decision.candidate_name,
            action_id: event.action_id,
            rollback_performed: event.rollback_performed,
            reason: event.reason,
        })
        .collect()
}

impl DaemonWatchSignature {
    fn from_output(output: &DaemonStatusOutput) -> Self {
        Self {
            phase: output.state.phase,
            active_action_id: output
                .state
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.action_id.clone()),
            rollback_action_id: output
                .state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.action_id.clone()),
            rollback_available: output
                .state
                .active_rollback
                .as_ref()
                .is_some_and(|rollback| rollback.rollback_available),
            fault_reason: output
                .state
                .faulted
                .as_ref()
                .map(|fault| fault.reason.clone()),
        }
    }
}

fn build_daemon_doctor_report() -> DaemonDoctorReport {
    let state_path = default_daemon_state_snapshot_path();
    let state_result = load_daemon_state(&state_path);
    let (state_load_ok, state) = match state_result {
        Ok(state) => (true, state),
        Err(_) => (false, DaemonState::default()),
    };
    let current_health = configured_system_health_snapshot();
    let capabilities = CapabilityProbe::default().probe();
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&state, &current_health),
        &DaemonWatchdogConfig::default(),
    );
    let state_uncertain = !state_load_ok;
    let safe_observe_only_required =
        state_uncertain || !current_health.ok_for_apply || !watchdog.ok;
    let mut checks = Vec::new();

    checks.push(DaemonDoctorCheck {
        name: "state_store_load".to_owned(),
        passed: state_load_ok,
        message: if state_load_ok {
            "daemon state snapshot loaded".to_owned()
        } else {
            "daemon state is missing or corrupt; apply should remain disabled until reset or recovery"
                .to_owned()
        },
    });
    checks.push(DaemonDoctorCheck {
        name: "health_ok_for_apply".to_owned(),
        passed: current_health.ok_for_apply,
        message: current_health
            .reason_code
            .clone()
            .unwrap_or_else(|| "health model permits apply".to_owned()),
    });
    checks.push(DaemonDoctorCheck {
        name: "watchdog_ok".to_owned(),
        passed: watchdog.ok,
        message: if watchdog.ok {
            "watchdog has no active safety issue".to_owned()
        } else {
            watchdog
                .issues
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "watchdog reported an unsafe state".to_owned())
        },
    });
    checks.push(DaemonDoctorCheck {
        name: "rollback_state".to_owned(),
        passed: state.active_rollback.is_none() || state.active_experiment.is_some(),
        message: if state.active_rollback.is_some() && state.active_experiment.is_none() {
            "rollback record exists without an active experiment".to_owned()
        } else {
            "rollback state is clean or intentionally active".to_owned()
        },
    });

    DaemonDoctorReport {
        state_path: state_path.display().to_string(),
        state_load_ok,
        state_uncertain,
        safe_observe_only_required,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        checks,
        capabilities,
        current_health,
        watchdog,
    }
}

fn reset_daemon_state(dry_run: bool) -> anyhow::Result<DaemonResetStateReport> {
    let state_path = default_daemon_state_snapshot_path();
    let state_exists = state_path.exists();
    let backup_path = state_exists
        .then(|| state_path.with_extension(format!("json.bak.{}", crate::audit::unix_nanos_now())));
    let reset_state = safe_reset_daemon_state();

    if !dry_run {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(backup_path) = backup_path.as_ref() {
            fs::copy(&state_path, backup_path)?;
        }
        DaemonStateSnapshotWriter::new(&state_path).write(&reset_state)?;
    }

    Ok(DaemonResetStateReport {
        state_path: state_path.display().to_string(),
        dry_run,
        state_exists,
        backup_path: backup_path.map(|path| path.display().to_string()),
        reset_state,
    })
}

fn safe_reset_daemon_state() -> DaemonState {
    DaemonState {
        mode: crate::daemon::DaemonMode::Observe,
        phase: DaemonPhase::Disabled,
        last_decision: Some(crate::daemon::DaemonDecisionState {
            decision: "daemon_state_reset".to_owned(),
            reason: "operator reset daemon state to safe observe-only defaults".to_owned(),
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        }),
        ..DaemonState::default()
    }
}

fn daemon_state_store_for_path(path: &Path) -> anyhow::Result<DaemonStateStore> {
    let state = if path.exists() {
        load_daemon_state(path)?
    } else {
        DaemonState::default()
    };

    Ok(DaemonStateStore::new(
        state,
        Some(DaemonStateSnapshotWriter::new(path)),
    ))
}

fn daemon_config_explain_descriptor() -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId("daemon-config-explain".to_owned()),
        action_kind: "daemon-config-explain".to_owned(),
        safety_class: SafetyClass::ObserveOnly,
        effect_scope: ActionEffectScope::ObserveOnly,
        rollback: RollbackRequirement::NotRequiredForDryRun,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: false,
        confidence: None,
    }
}

fn render_config_explain_json(output: &DaemonConfigExplainOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

fn render_policy_explain_json(output: &DaemonPolicyExplainOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

fn render_profiles_list_text(output: &DaemonProfilesListOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("profiles: {}\n", output.profiles.len()));

    if output.profiles.is_empty() {
        text.push_str("profile: none\n");
    } else {
        for profile in &output.profiles {
            text.push_str(&format!(
                "profile workload_hash={} label={} candidate={} action_id={} safety_class={:?} confidence_milli={} kept_unix_nanos={}\n",
                profile.workload_identity_hash,
                profile.workload_label.as_deref().unwrap_or("-"),
                profile.candidate_name,
                profile.action_id,
                profile.safety_class,
                profile.confidence_milli,
                profile.kept_unix_nanos
            ));
        }
    }

    text
}

fn render_profiles_forget_text(output: &DaemonProfilesForgetOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles forget\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("dry_run: {}\n", output.dry_run));
    text.push_str(&format!("before_count: {}\n", output.before_count));
    text.push_str(&format!("removed_count: {}\n", output.removed_count));
    text.push_str(&format!("remaining_count: {}\n", output.remaining_count));
    for profile in &output.removed {
        text.push_str(&format!(
            "removed workload_hash={} candidate={} action_id={}\n",
            profile.workload_identity_hash, profile.candidate_name, profile.action_id
        ));
    }
    text
}

fn render_profiles_explain_text(output: &DaemonProfilesExplainOutput) -> String {
    let mut text = String::new();
    text.push_str("Daemon profiles explain\n");
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("profiles: {}\n", output.profiles.len()));
    text.push_str(&format!(
        "current_kernel: {}\n",
        output
            .current_environment
            .kernel_version
            .as_deref()
            .unwrap_or("-")
    ));
    text.push_str(&format!(
        "current_cpu_topology_hash: {}\n",
        output
            .current_environment
            .cpu_topology_hash
            .as_deref()
            .unwrap_or("-")
    ));

    if output.profiles.is_empty() {
        text.push_str("profile: none\n");
    } else {
        for explanation in &output.profiles {
            let reasons = if explanation.validation.reason_codes.is_empty() {
                "none".to_owned()
            } else {
                explanation.validation.reason_codes.join(",")
            };
            text.push_str(&format!(
                "profile workload_hash={} candidate={} valid={} confidence_milli={} reason_codes={}\n",
                explanation.profile.workload_identity_hash,
                explanation.profile.candidate_name,
                explanation.validation.valid,
                explanation.validation.confidence_milli,
                reasons
            ));
        }
    }

    text
}

fn render_explain_text(output: &DaemonExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon explain\n");
    text.push_str("==============\n");
    text.push_str(&format!("state_loaded: {}\n", output.status.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.status.state_path));
    text.push_str(&format!("mode: {}\n", output.status.state.mode));
    text.push_str(&format!(
        "phase: {}\n",
        output.status.state.phase.lifecycle_label()
    ));
    text.push_str(&format!(
        "health: {}\n",
        output.status.current_health.state.as_str()
    ));
    text.push_str(&format!("watchdog_ok: {}\n", output.status.watchdog.ok));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.status.manual_restore_command
    ));

    text.push_str("\nWhy no optimize\n");
    text.push_str("----------------\n");
    for reason in &output.status_explanation.why_no_optimize {
        text.push_str(&format!("- {reason}\n"));
    }

    text.push_str("\nWhat changed\n");
    text.push_str("------------\n");
    for change in &output.status_explanation.what_changed {
        text.push_str(&format!("- {change}\n"));
    }

    text.push_str("\nPolicy decisions\n");
    text.push_str("----------------\n");
    for line in &output.policy_explanation.lines {
        text.push_str(&format!(
            "- {}: {} - {}\n",
            line.rule, line.outcome, line.reason
        ));
    }

    if !output.status.recent_decisions.is_empty() {
        text.push_str("\nRecent decisions\n");
        text.push_str("----------------\n");
        for decision in &output.status.recent_decisions {
            text.push_str(&format!(
                "- {} candidate={} action={} rollback={} reason={}\n",
                decision.decision,
                decision.candidate_name.as_deref().unwrap_or("none"),
                decision.action_id.as_deref().unwrap_or("none"),
                decision.rollback_performed,
                decision.reason
            ));
        }
    }

    text
}

fn render_why_not_optimize_text(output: &DaemonWhyNotOptimizeOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon why-not-optimize\n");
    text.push_str("=======================\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.mode));
    text.push_str(&format!("phase: {}\n", output.phase.lifecycle_label()));
    text.push_str(&format!("health: {}\n", output.health.state.as_str()));
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.manual_restore_command
    ));
    text.push_str("reasons:\n");
    for reason in &output.why_no_optimize {
        text.push_str(&format!("- {reason}\n"));
    }

    if !output.recent_decisions.is_empty() {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "- {} reason={}\n",
                decision.decision, decision.reason
            ));
        }
    }

    text
}

fn render_what_changed_text(output: &DaemonWhatChangedOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon what-changed\n");
    text.push_str("===================\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.mode));
    text.push_str(&format!("phase: {}\n", output.phase.lifecycle_label()));
    text.push_str(&format!("health: {}\n", output.health.state.as_str()));
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    text.push_str("changes:\n");
    for change in &output.what_changed {
        text.push_str(&format!("- {change}\n"));
    }

    if !output.recent_decisions.is_empty() {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "- {} reason={}\n",
                decision.decision, decision.reason
            ));
        }
    }

    text
}

fn render_status_json(output: &DaemonStatusOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

fn render_bench_overhead_text(report: &DaemonOverheadReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon overhead benchmark\n");
    text.push_str("=========================\n");
    text.push_str(&format!("within_budget: {}\n", report.within_budget));
    text.push_str(&format!(
        "sample_duration_millis: {}\n",
        report.snapshot.sample_duration_millis
    ));
    text.push_str(&format!(
        "cpu_millis_per_second: {} / {}\n",
        report.snapshot.cpu_millis_per_second, report.budget.max_cpu_millis_per_second
    ));
    if let Some(bytes) = report.snapshot.memory_rss_bytes {
        text.push_str(&format!(
            "memory_rss_bytes: {} / {}\n",
            bytes, report.budget.max_memory_bytes
        ));
    }
    if let Some(fds) = report.snapshot.open_fds {
        text.push_str(&format!(
            "open_fds: {} / {}\n",
            fds, report.budget.max_open_fds
        ));
    }
    if let Some(bytes) = report.snapshot.disk_write_bytes_per_minute {
        text.push_str(&format!(
            "disk_write_bytes_per_minute: {} / {}\n",
            bytes, report.budget.max_disk_write_bytes_per_minute
        ));
    }
    for issue in &report.issues {
        text.push_str(&format!(
            "issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }

    text
}

fn render_soak_text(report: &DaemonSoakReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon scenario soak\n");
    text.push_str("====================\n");
    text.push_str(&format!("profile: {}\n", report.profile));
    text.push_str(&format!("duration_seconds: {}\n", report.duration_seconds));
    text.push_str(&format!("ticks: {}\n", report.ticks));
    text.push_str(&format!("passed: {}\n", report.passed));
    text.push_str(&format!(
        "scenario_count: {}\n",
        report.metrics.scenario_count
    ));
    text.push_str(&format!(
        "planner_decisions: {}\n",
        report.metrics.planner_decisions
    ));
    text.push_str(&format!(
        "memory_growth_bytes: {}\n",
        report.metrics.memory_growth_bytes
    ));
    text.push_str(&format!(
        "disk_growth_bytes: {}\n",
        report.metrics.disk_growth_bytes
    ));
    text.push_str(&format!(
        "max_event_queue_len: {}\n",
        report.metrics.max_event_queue_len
    ));
    text.push_str(&format!("task_count: {}\n", report.metrics.task_count));
    text.push_str(&format!(
        "history_bytes: {}\n",
        report.metrics.history_bytes
    ));
    text.push_str(&format!(
        "cpu_millis_per_second: {}\n",
        report.metrics.cpu_millis_per_second
    ));
    text.push_str(&format!(
        "wakeups_per_second: {}\n",
        report.metrics.wakeups_per_second
    ));
    text.push_str(&format!("event_drops: {}\n", report.metrics.event_drops));
    text.push_str(&format!(
        "fake_actions_started: {}\n",
        report.metrics.fake_actions_started
    ));
    text.push_str(&format!(
        "fake_rollbacks: {}\n",
        report.metrics.fake_rollbacks
    ));
    text.push_str(&format!(
        "max_active_experiments: {}\n",
        report.metrics.max_active_experiments
    ));
    for scenario in &report.scenarios {
        text.push_str(&format!(
            "scenario: {} mode={} ticks={} passed={} decisions={}\n",
            scenario.name,
            scenario.mode,
            scenario.ticks,
            scenario.passed,
            scenario.decisions.join(",")
        ));
    }
    for failure in &report.failures {
        text.push_str(&format!(
            "failure: {} - {}\n",
            failure.reason_code, failure.message
        ));
    }

    text
}

fn render_acceptance_text(report: &DaemonAcceptanceReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon acceptance\n");
    text.push_str("=================\n");
    text.push_str(&format!("suite: {}\n", report.suite));
    text.push_str(&format!("passed: {}\n", report.passed));

    for step in &report.steps {
        text.push_str(&format!(
            "step {} {}: {} - {}\n",
            step.number,
            step.code,
            if step.passed { "passed" } else { "failed" },
            step.evidence
        ));
    }

    text
}

fn render_daemon_doctor_text(report: &DaemonDoctorReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon doctor\n");
    text.push_str("=============\n");
    text.push_str(&format!("state_path: {}\n", report.state_path));
    text.push_str(&format!("state_load_ok: {}\n", report.state_load_ok));
    text.push_str(&format!("state_uncertain: {}\n", report.state_uncertain));
    text.push_str(&format!(
        "safe_observe_only_required: {}\n",
        report.safe_observe_only_required
    ));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        report.manual_restore_command
    ));
    text.push_str(&format!(
        "health: {}\n",
        report.current_health.state.as_str()
    ));
    text.push_str(&format!(
        "health_ok_for_apply: {}\n",
        report.current_health.ok_for_apply
    ));
    if let Some(reason) = report.current_health.reason_code.as_ref() {
        text.push_str(&format!("health_reason: {reason}\n"));
    }
    for issue in &report.current_health.issues {
        text.push_str(&format!(
            "health_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    text.push_str(&format!("watchdog_ok: {}\n", report.watchdog.ok));
    if report.watchdog.recommended_actions.is_empty() {
        text.push_str("watchdog_actions: none\n");
    } else {
        text.push_str(&format!(
            "watchdog_actions: {}\n",
            report
                .watchdog
                .recommended_actions
                .iter()
                .map(|action| action.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for issue in &report.watchdog.issues {
        text.push_str(&format!(
            "watchdog_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }

    text.push_str(&format!(
        "kernel_release: {}\n",
        report
            .capabilities
            .kernel_release
            .as_deref()
            .unwrap_or("unknown")
    ));
    let unavailable = report.capabilities.unavailable_features();
    if unavailable.is_empty() {
        text.push_str("unavailable_features: none\n");
    } else {
        text.push_str(&format!(
            "unavailable_features: {}\n",
            unavailable.join(", ")
        ));
    }

    text.push_str("checks:\n");
    for check in &report.checks {
        let status = if check.passed { "passed" } else { "failed" };
        text.push_str(&format!(
            "  - {}: {} - {}\n",
            check.name, status, check.message
        ));
    }

    text
}

fn render_reset_state_text(report: &DaemonResetStateReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon reset-state\n");
    text.push_str("==================\n");
    text.push_str(&format!("dry_run: {}\n", report.dry_run));
    text.push_str(&format!("state_path: {}\n", report.state_path));
    text.push_str(&format!("state_exists: {}\n", report.state_exists));
    text.push_str(&format!(
        "backup_path: {}\n",
        report.backup_path.as_deref().unwrap_or("none")
    ));
    text.push_str(&format!("reset_mode: {}\n", report.reset_state.mode));
    text.push_str(&format!(
        "reset_phase: {}\n",
        report.reset_state.phase.lifecycle_label()
    ));
    if let Some(decision) = report.reset_state.last_decision.as_ref() {
        text.push_str(&format!("reset_decision: {}\n", decision.decision));
        text.push_str(&format!("reset_reason: {}\n", decision.reason));
    }
    if report.dry_run {
        text.push_str("result: no changes written\n");
    } else {
        text.push_str("result: daemon state reset written\n");
    }

    text
}

fn render_config_explain_text(output: &DaemonConfigExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon config explanation\n");
    text.push_str("=========================\n");
    text.push_str(&format!(
        "user_config_loaded: {}\n",
        output.user_config_loaded
    ));
    text.push_str(&format!("preset: {}\n", output.config.preset));
    text.push_str(&format!("mode: {}\n", output.config.mode));
    text.push_str(&format!("source: {:?}\n", output.config.source));
    text.push_str(&format!(
        "target_pids: {:?}\n",
        output.config.target.target_pids
    ));
    text.push_str(&format!(
        "tree_pids: {:?}\n",
        output.config.target.tree_pids
    ));
    text.push_str(&format!(
        "watch_process: {}\n",
        output
            .config
            .target
            .watch_process
            .as_deref()
            .unwrap_or("<none>")
    ));
    text.push_str(&format!(
        "require_explicit_target: {}\n",
        output.config.target.require_explicit_target
    ));
    text.push_str("\nEffective policy\n");
    text.push_str("----------------\n");
    text.push_str(&format!(
        "max_safety_class: {:?}\n",
        output.policy.max_safety_class
    ));
    text.push_str(&format!(
        "rollback_required_before_apply: {}\n",
        output.policy.rollback_required_before_apply
    ));
    text.push_str(&format!(
        "allow_system_wide_suggestions: {}\n",
        output.policy.allow_system_wide_suggestions
    ));
    text.push_str(&format!(
        "allow_system_wide_apply: {}\n",
        output.policy.allow_system_wide_apply
    ));
    text.push_str(&format!(
        "allow_high_risk: {}\n",
        output.policy.allow_high_risk
    ));
    text.push_str(&format!(
        "allow_persistent_effects: {}\n",
        output.policy.allow_persistent_effects
    ));
    text.push_str(&format!(
        "allow_cpu_power_on_battery: {}\n",
        output.policy.allow_cpu_power_on_battery
    ));
    text.push_str(&format!(
        "enabled_action_families: {}\n",
        if output.policy.enabled_action_families.is_empty() {
            "none".to_owned()
        } else {
            output
                .policy
                .enabled_action_families
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    text.push_str(&format!(
        "denied_action_families: {}\n",
        if output.policy.denied_action_families.is_empty() {
            "none".to_owned()
        } else {
            output
                .policy
                .denied_action_families
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    text.push_str(&format!(
        "min_confidence: {:.3}\n",
        output.policy.min_confidence
    ));
    text.push_str("\nHealth guardrails\n");
    text.push_str("-----------------\n");
    text.push_str(&format!(
        "max_cpu_temp_celsius: {}\n",
        output.config.health.max_cpu_temp_celsius
    ));
    text.push_str(&format!(
        "max_gpu_temp_celsius: {}\n",
        output.config.health.max_gpu_temp_celsius
    ));
    text.push_str(&format!(
        "min_disk_available_bytes: {}\n",
        output.config.health.min_disk_available_bytes
    ));
    text.push_str(&format!(
        "max_memory_pressure_some_avg10_percent: {:.3}\n",
        output.config.health.max_memory_pressure_some_avg10_percent
    ));
    text.push_str(&format!(
        "remote_apply_allowed: {}\n",
        output.policy.remote_apply.allow_remote_apply
    ));
    text.push_str(&format!(
        "agent_limits.max_mode: {}\n",
        output.agent_autotune_limits.max_mode
    ));
    text.push_str(&format!(
        "agent_limits.max_safety_class: {:?}\n",
        output.agent_autotune_limits.max_safety_class
    ));
    text.push_str(&format!(
        "agent_limits.max_targets: {}\n",
        output.agent_autotune_limits.max_targets
    ));
    text.push_str("\nExplanation\n");
    text.push_str("-----------\n");
    text.push_str(&format!(
        "verdict: {}\n",
        output.explanation.verdict.as_str()
    ));
    text.push_str(&format!("decision: {:?}\n", output.explanation.decision));
    text.push_str(&format!("intent: {:?}\n", output.explanation.intent));
    text.push_str(&format!(
        "final_reason: {}\n",
        output.explanation.final_reason
    ));
    text.push_str("evaluated_rules:\n");

    for rule in &output.explanation.evaluated_rules {
        let status = if rule.passed { "passed" } else { "failed" };
        text.push_str(&format!(
            "  - {}: {} - {}\n",
            rule.rule, status, rule.reason
        ));
    }

    text
}

fn render_policy_explain_text(output: &DaemonPolicyExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon policy explanation\n");
    text.push_str("=========================\n");
    text.push_str(&format!(
        "user_config_loaded: {}\n",
        output.user_config_loaded
    ));
    text.push_str(&format!("preset: {}\n", output.config.preset));
    text.push_str(&format!("mode: {}\n", output.policy.mode));
    text.push_str(&format!("source: {:?}\n", output.policy.source));
    text.push_str(&format!(
        "max_safety_class: {:?}\n",
        output.policy.max_safety_class
    ));
    text.push_str(&format!(
        "min_confidence: {:.3}\n",
        output.policy.min_confidence
    ));
    text.push_str("\nPolicy decisions\n");
    text.push_str("----------------\n");

    for line in &output.explanation.lines {
        text.push_str(&format!(
            "- {}: {} - {}\n",
            line.rule, line.outcome, line.reason
        ));
    }

    text
}

fn render_status_text(output: &DaemonStatusOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon status\n");
    text.push_str("=============\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.state.mode));
    text.push_str(&format!(
        "phase: {}\n",
        output.state.phase.lifecycle_label()
    ));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.manual_restore_command
    ));
    if let Some(target) = output.state.active_target.as_ref() {
        text.push_str(&format!(
            "active_workload: root_pid={} comm={} targets={}\n",
            target
                .root_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            target.comm.as_deref().unwrap_or("unknown"),
            target.active_targets
        ));
    } else {
        text.push_str("active_workload: none\n");
    }
    if let Some(experiment) = output.state.active_experiment.as_ref() {
        text.push_str(&format!(
            "active_action: action_id={} candidate={} mode={} safety_class={:?}\n",
            experiment.action_id,
            experiment.candidate_name.as_deref().unwrap_or("unknown"),
            experiment.mode,
            experiment.safety_class
        ));
    } else {
        text.push_str("active_action: none\n");
    }
    if let Some(rollback) = output.state.active_rollback.as_ref() {
        text.push_str(&format!(
            "rollback_status: action_id={} mode={} safety_class={:?} available={} manual_restore_command={}\n",
            rollback.action_id,
            rollback.mode,
            rollback.safety_class,
            rollback.rollback_available,
            rollback
                .manual_restore_command
                .as_deref()
                .unwrap_or("unknown")
        ));
    } else {
        text.push_str("rollback_status: none\n");
    }
    if let Some(cooldown_until) = output.state.cooldown_until_unix_nanos {
        text.push_str(&format!("cooldown_until_unix_nanos: {cooldown_until}\n"));
    }
    text.push_str(&format!(
        "health: {}\n",
        output.current_health.state.as_str()
    ));
    text.push_str(&format!(
        "health_ok_for_apply: {}\n",
        output.current_health.ok_for_apply
    ));
    if let Some(reason) = output.current_health.reason_code.as_ref() {
        text.push_str(&format!("health_reason: {reason}\n"));
    }
    for issue in &output.current_health.issues {
        text.push_str(&format!(
            "health_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    if output.watchdog.recommended_actions.is_empty() {
        text.push_str("watchdog_actions: none\n");
    } else {
        text.push_str(&format!(
            "watchdog_actions: {}\n",
            output
                .watchdog
                .recommended_actions
                .iter()
                .map(|action| action.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for issue in &output.watchdog.issues {
        text.push_str(&format!(
            "watchdog_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    if let Some(decision) = output.state.last_decision.as_ref() {
        text.push_str(&format!("last_decision: {}\n", decision.decision));
        text.push_str(&format!("last_reason: {}\n", decision.reason));
        if let Some(score) = decision.score_total {
            text.push_str(&format!("current_score: {score}\n"));
        }
    }
    if let Some(fault) = output.state.faulted.as_ref() {
        text.push_str(&format!("fault: {}\n", fault.reason));
    }
    if output.state.phase == DaemonPhase::Paused {
        text.push_str("pause_state: operator_paused\n");
    }
    let unavailable = output.capabilities.unavailable_features();
    if unavailable.is_empty() {
        text.push_str("unavailable_features: none\n");
    } else {
        text.push_str(&format!(
            "unavailable_features: {}\n",
            unavailable.join(", ")
        ));
    }

    if output.recent_decisions.is_empty() {
        text.push_str("recent_decisions: none\n");
    } else {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "  - unix_nanos={} mode={} phase={} decision={} action={} candidate={} rollback_performed={} reason={}\n",
                decision.unix_nanos,
                decision.mode,
                decision.phase,
                decision.decision,
                decision.action_id.as_deref().unwrap_or("none"),
                decision.candidate_name.as_deref().unwrap_or("none"),
                decision.rollback_performed,
                decision.reason
            ));
        }
    }

    text
}

fn render_watch_line(output: &DaemonStatusOutput) -> String {
    let workload = output
        .state
        .active_target
        .as_ref()
        .map(|target| {
            format!(
                "{}:{}",
                target.comm.as_deref().unwrap_or("unknown"),
                target
                    .root_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            )
        })
        .unwrap_or_else(|| "none".to_owned());
    let action = output
        .state
        .active_experiment
        .as_ref()
        .map(|experiment| experiment.action_id.as_str())
        .unwrap_or("none");
    let rollback = output
        .state
        .active_rollback
        .as_ref()
        .map(|rollback| {
            if rollback.rollback_available {
                "available"
            } else {
                "restore-needed"
            }
        })
        .unwrap_or("none");
    let last = output
        .state
        .last_decision
        .as_ref()
        .map(|decision| decision.decision.as_str())
        .unwrap_or("none");

    format!(
        "daemon mode={} phase={} health={} watchdog_ok={} workload={} action={} rollback={} last_decision={} restore=\"{}\"\n",
        output.state.mode,
        output.state.phase.lifecycle_label(),
        output.current_health.state.as_str(),
        output.watchdog.ok,
        workload,
        action,
        rollback,
        last,
        output.manual_restore_command
    )
}

fn render_watch_notification(
    previous: &DaemonWatchSignature,
    current: &DaemonWatchSignature,
) -> Option<String> {
    if current.fault_reason != previous.fault_reason
        && let Some(reason) = current.fault_reason.as_ref()
    {
        return Some(format!("fault: {reason}"));
    }

    if current.rollback_action_id != previous.rollback_action_id
        && let Some(action_id) = current.rollback_action_id.as_ref()
    {
        if current.rollback_available {
            return Some(format!("rollback available for action_id={action_id}"));
        }
        return Some(format!("restore needed for action_id={action_id}"));
    }

    if previous.rollback_available
        && !current.rollback_available
        && let Some(action_id) = current.rollback_action_id.as_ref()
    {
        return Some(format!("restore needed for action_id={action_id}"));
    }

    if current.phase == DaemonPhase::Rollback && previous.phase != DaemonPhase::Rollback {
        return Some("rollback started".to_owned());
    }

    if current.active_action_id != previous.active_action_id
        && let Some(action_id) = current.active_action_id.as_ref()
    {
        return Some(format!("action applied action_id={action_id}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-daemon-command-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn daemon_test_profile(workload: &str, candidate: &str) -> DaemonWorkloadProfile {
        DaemonWorkloadProfile {
            workload_identity_hash: workload.to_owned(),
            workload_label: Some("game".to_owned()),
            candidate_name: candidate.to_owned(),
            action_id: format!("cpu-affinity-profile:{candidate}"),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            kept_unix_nanos: 1_000,
            last_validated_unix_nanos: Some(1_000),
            baseline_score_total: Some(1_000),
            candidate_score_total: Some(850),
            score_delta: -150,
            confidence_milli: 900,
            environment: DaemonProfileEnvironment {
                hardware_fingerprint: Some("hardware-a".to_owned()),
                kernel_version: Some("6.12.0".to_owned()),
                cpu_topology_hash: Some("topology-a".to_owned()),
                scheduler_label: Some("scx_lavd".to_owned()),
                scx_ops: Some("scx_lavd".to_owned()),
                ..DaemonProfileEnvironment::default()
            },
            partition: crate::daemon::DaemonProfilePartition {
                power_source: Some("ac".to_owned()),
                scheduler_label: Some("scx_lavd".to_owned()),
                ..crate::daemon::DaemonProfilePartition::default()
            },
        }
    }

    #[test]
    fn daemon_config_explain_text_contains_effective_policy_and_rules() {
        let output = build_config_explain_output_from_user_config(None, None).unwrap();

        let text = render_config_explain_text(&output);

        assert!(text.contains("Daemon config explanation"));
        assert!(text.contains("Effective policy"));
        assert!(text.contains("preset: observe-only"));
        assert!(text.contains("mode: observe"));
        assert!(text.contains("max_safety_class: ObserveOnly"));
        assert!(text.contains("verdict: allow"));
        assert!(text.contains("final_reason: action is allowed by daemon policy"));
        assert!(text.contains("intent_allowed"));
    }

    #[test]
    fn daemon_config_explain_json_contains_config_policy_and_explanation() {
        let output = build_config_explain_output_from_user_config(None, None).unwrap();

        let json = render_config_explain_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["config"]["mode"], "observe");
        assert_eq!(value["config"]["preset"], "observe-only");
        assert_eq!(value["config"]["source"], "cli");
        assert_eq!(value["policy"]["mode"], "observe");
        assert_eq!(value["explanation"]["action_kind"], "daemon-config-explain");
        assert_eq!(value["explanation"]["verdict"], "allow");
        assert_eq!(
            value["explanation"]["final_reason"],
            "action is allowed by daemon policy"
        );
        assert_eq!(value["agent_autotune_limits"]["max_mode"], "apply-low-risk");
    }

    #[test]
    fn daemon_policy_explain_text_contains_action_level_decisions() {
        let output =
            build_policy_explain_output_from_user_config(None, Some("gaming-low-risk")).unwrap();

        let text = render_policy_explain_text(&output);

        assert!(text.contains("Daemon policy explanation"));
        assert!(text.contains("preset: gaming-low-risk"));
        assert!(text.contains("action:apply_low_risk_cpu_affinity: allowed"));
        assert!(text.contains("action:apply_without_rollback:rollback_available: failed"));
        assert!(!text.contains("later patch"));
    }

    #[test]
    fn daemon_policy_explain_json_contains_policy_lines() {
        let output = build_policy_explain_output_from_user_config(None, None).unwrap();

        let json = render_policy_explain_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["config"]["preset"], "observe-only");
        assert_eq!(value["policy"]["mode"], "observe");
        let lines = value["explanation"]["lines"].as_array().unwrap();
        assert!(lines.iter().any(|line| {
            line["rule"] == "action:observe_status" && line["outcome"] == "allowed"
        }));
        assert!(lines.iter().any(|line| {
            line["rule"] == "action:apply_low_risk_cpu_affinity"
                && line["outcome"] == "rejected:intent_not_allowed"
        }));
    }

    #[test]
    fn daemon_profiles_list_text_reports_persistent_profiles() {
        let state = DaemonState {
            profile_memory: crate::daemon::DaemonProfileMemory {
                profiles: vec![daemon_test_profile("workload-a", "game-main")],
            },
            ..DaemonState::default()
        };
        let output = build_profiles_list_output_from_state("state.json".to_owned(), true, &state);

        let text = render_profiles_list_text(&output);

        assert!(text.contains("Daemon profiles"));
        assert!(text.contains("profiles: 1"));
        assert!(text.contains("workload_hash=workload-a"));
        assert!(text.contains("candidate=game-main"));
    }

    #[test]
    fn daemon_profiles_explain_text_reports_invalidation_reasons() {
        let state = DaemonState {
            profile_memory: crate::daemon::DaemonProfileMemory {
                profiles: vec![daemon_test_profile("workload-a", "game-main")],
            },
            ..DaemonState::default()
        };
        let current_environment = DaemonProfileEnvironment {
            hardware_fingerprint: Some("hardware-a".to_owned()),
            kernel_version: Some("6.13.0".to_owned()),
            cpu_topology_hash: Some("topology-b".to_owned()),
            scheduler_label: Some("scx_bpfland".to_owned()),
            scx_ops: Some("scx_bpfland".to_owned()),
            ..DaemonProfileEnvironment::default()
        };
        let output = build_profiles_explain_output_from_state(
            "state.json".to_owned(),
            true,
            &state,
            Some("workload-a"),
            current_environment,
            1_000,
        );

        let text = render_profiles_explain_text(&output);

        assert!(text.contains("Daemon profiles explain"));
        assert!(text.contains("valid=false"));
        assert!(text.contains("kernel_changed"));
        assert!(text.contains("cpu_topology_changed"));
    }

    #[test]
    fn daemon_profiles_forget_text_summarizes_removed_profiles() {
        let output = DaemonProfilesForgetOutput {
            state_path: "state.json".to_owned(),
            dry_run: true,
            before_count: 2,
            removed_count: 1,
            remaining_count: 1,
            removed: vec![daemon_test_profile("workload-a", "game-main")],
        };

        let text = render_profiles_forget_text(&output);

        assert!(text.contains("Daemon profiles forget"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains("removed_count: 1"));
        assert!(text.contains("removed workload_hash=workload-a candidate=game-main"));
    }

    #[test]
    fn daemon_config_explain_loads_agent_limits_from_user_config() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-laptop-safe".to_owned()),
            agent: Some(crate::config_file::AgentConfigFile {
                autotune_limits: Some(crate::config_file::AgentAutotuneLimitsFile {
                    max_candidate_window_seconds: Some(60),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), None).unwrap();

        assert!(output.user_config_loaded);
        assert_eq!(output.config.preset, DaemonPreset::GamingLaptopSafe);
        assert_eq!(
            output.agent_autotune_limits.max_candidate_window_seconds,
            60
        );
    }

    #[test]
    fn daemon_config_explain_applies_safe_user_policy_overrides() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            daemon_enabled_action_families: Some(vec!["cpu_affinity_profile".to_owned()]),
            daemon_denied_action_families: Some(vec!["ionice".to_owned()]),
            daemon_min_confidence: Some(0.93),
            daemon_max_cpu_temp_celsius: Some(81),
            daemon_max_gpu_temp_celsius: Some(82),
            daemon_min_disk_available_bytes: Some(2_000_000_000),
            daemon_max_memory_pressure_some_avg10_percent: Some(15.5),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), None).unwrap();

        assert!(
            output
                .policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(output.policy.denied_action_families.contains("ionice"));
        assert_eq!(output.policy.min_confidence, 0.93);
        assert_eq!(output.config.health.max_cpu_temp_celsius, 81);
        assert_eq!(output.config.health.max_gpu_temp_celsius, 82);
        assert_eq!(output.config.health.min_disk_available_bytes, 2_000_000_000);
        assert_eq!(
            output
                .config
                .health
                .thresholds()
                .max_memory_pressure_some_avg10_millipercent,
            15_500
        );

        let text = render_config_explain_text(&output);
        assert!(text.contains("denied_action_families: ionice"));
        assert!(text.contains("min_confidence: 0.930"));
        assert!(text.contains("max_cpu_temp_celsius: 81"));
        assert!(text.contains("min_disk_available_bytes: 2000000000"));
    }

    #[test]
    fn daemon_explain_policy_uses_configured_safety_with_live_state() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            daemon_denied_action_families: Some(vec!["ionice".to_owned()]),
            daemon_min_confidence: Some(0.93),
            ..Default::default()
        };
        let state = DaemonState {
            mode: crate::daemon::DaemonMode::ApplyLowRisk,
            active_target: Some(crate::daemon::DaemonTargetState {
                root_pid: Some(4242),
                active_targets: 1,
                comm: Some("Game.exe".to_owned()),
            }),
            ..DaemonState::default()
        };

        let config = build_daemon_config_from_state(&state, true, Some(&user_config)).unwrap();
        let policy = build_policy_from_daemon_state_with_user_config_result(
            &state,
            true,
            Ok(Some(user_config)),
        );

        assert_eq!(config.mode, crate::daemon::DaemonMode::ApplyLowRisk);
        assert_eq!(config.target.tree_pids, vec![4242]);
        assert_eq!(config.target.watch_process.as_deref(), Some("Game.exe"));
        assert!(config.target.require_explicit_target);
        assert!(
            policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(policy.denied_action_families.contains("ionice"));
        assert_eq!(policy.min_confidence, 0.93);
    }

    #[test]
    fn daemon_explain_policy_uses_configured_mode_when_state_is_missing() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            ..Default::default()
        };

        let policy = build_policy_from_daemon_state_with_user_config_result(
            &DaemonState::default(),
            false,
            Ok(Some(user_config)),
        );

        assert_eq!(policy.mode, crate::daemon::DaemonMode::ApplyLowRisk);
        assert!(
            policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
    }

    #[test]
    fn daemon_explain_policy_falls_back_to_observe_only_when_config_is_unreadable() {
        let state = DaemonState {
            mode: crate::daemon::DaemonMode::ApplyLowRisk,
            ..DaemonState::default()
        };

        let policy = build_policy_from_daemon_state_with_user_config_result(
            &state,
            true,
            Err(anyhow::anyhow!("broken daemon config")),
        );

        assert_eq!(policy.mode, crate::daemon::DaemonMode::Observe);
        assert_eq!(policy.max_safety_class, SafetyClass::ObserveOnly);
        assert!(!policy.remote_apply.allow_remote_apply);
    }

    #[test]
    fn daemon_status_health_monitor_uses_configured_guardrails() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-laptop-safe".to_owned()),
            daemon_max_cpu_temp_celsius: Some(70),
            daemon_max_gpu_temp_celsius: Some(71),
            daemon_min_disk_available_bytes: Some(3_000_000_000),
            daemon_max_memory_pressure_some_avg10_percent: Some(12.25),
            ..Default::default()
        };

        let monitor = system_health_monitor_from_user_config(Some(&user_config)).unwrap();

        assert_eq!(monitor.thresholds().max_cpu_temp_millidegrees, 70_000);
        assert_eq!(monitor.thresholds().max_gpu_temp_millidegrees, 71_000);
        assert_eq!(monitor.thresholds().min_disk_available_bytes, 3_000_000_000);
        assert_eq!(
            monitor
                .thresholds()
                .max_memory_pressure_some_avg10_millipercent,
            12_250
        );
    }

    #[test]
    fn daemon_status_health_blocks_apply_when_config_is_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let root = SystemHealthProbeRoot {
            proc_root: temp.path().join("proc"),
            sys_root: temp.path().join("sys"),
            disk_path: temp.path().to_path_buf(),
        };

        let health = system_health_snapshot_from_user_config_result(
            Err(anyhow::anyhow!("invalid daemon config")),
            root,
        );

        assert_eq!(
            health.state,
            crate::daemon::SystemHealthState::InstrumentationBroken
        );
        assert!(!health.ok_for_apply);
        assert_eq!(
            health.reason_code.as_deref(),
            Some("instrumentation_probe_failed")
        );
        assert!(health.inputs.probe_errors.iter().any(|error| {
            error.contains("daemon_config_load_failed") && error.contains("invalid daemon config")
        }));

        let output = DaemonStatusOutput {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_loaded: true,
            state: DaemonState::default(),
            capabilities: DaemonCapabilities {
                kernel_release: None,
                btf_available: false,
                sched_tracepoints_available: false,
                perf_permissions_likely: false,
                perf_event_paranoid: None,
                cgroup_v2_available: false,
                sched_ext_available: false,
                uclamp_available: false,
                ionice_available: true,
                irq_affinity_available: false,
                gpu_sysfs_available: false,
            },
            watchdog: evaluate_daemon_watchdog(
                DaemonWatchdogInputs::from_state_and_health(&DaemonState::default(), &health),
                &DaemonWatchdogConfig::default(),
            ),
            current_health: health,
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            recent_decisions: Vec::new(),
        };
        let text = render_status_text(&output);

        assert!(text.contains("health: instrumentation_broken"));
        assert!(text.contains("health_ok_for_apply: false"));
        assert!(text.contains("health_issue: instrumentation_probe_failed"));
    }

    #[test]
    fn daemon_config_explain_rejects_unguarded_experimental_policy_overrides() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            daemon_allow_system_wide_suggestions: Some(true),
            daemon_allow_system_wide_apply: Some(true),
            ..Default::default()
        };

        let err = build_config_explain_output_from_user_config(Some(&user_config), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("experimental = true"));
    }

    #[test]
    fn daemon_config_explain_cli_preset_overrides_user_file_preset() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-laptop-safe".to_owned()),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), Some("observe-only"))
                .unwrap();

        assert_eq!(output.config.preset, DaemonPreset::ObserveOnly);
        assert_eq!(output.config.mode, crate::daemon::DaemonMode::Observe);
    }

    #[test]
    fn daemon_config_explain_can_render_low_risk_preset() {
        let output =
            build_config_explain_output_from_user_config(None, Some("gaming-low-risk")).unwrap();

        assert_eq!(output.config.preset, DaemonPreset::GamingLowRisk);
        assert_eq!(output.config.mode, crate::daemon::DaemonMode::ApplyLowRisk);
        assert!(
            output
                .policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(output.policy.min_confidence >= 0.85);

        let text = render_config_explain_text(&output);
        assert!(text.contains("preset: gaming-low-risk"));
        assert!(text.contains("enabled_action_families: cpu_affinity_profile"));
    }

    #[test]
    fn daemon_restore_dry_run_discovers_autotune_and_profile_restore_records() {
        let dir = temp_dir("restore-dry-run");
        let journal_path = dir.join("controller_journal.json");
        let audit_path = dir.join("audit.jsonl");
        let history_path = dir.join("history.jsonl");
        let affinity_path = dir.join("last_affinity_restore.json");
        let profile_path = dir.join("last_profile_restore.json");

        crate::autotune::controller_journal::write_controller_journal_clean(&journal_path).unwrap();
        crate::affinity::save_restore_state(
            &affinity_path,
            &[crate::affinity::AffinityRecord {
                tid: 123,
                process_pid: Some(123),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                original_mask: crate::affinity::CpuMask::parse("0").unwrap(),
                applied_mask: crate::affinity::CpuMask::parse("1").unwrap(),
            }],
        )
        .unwrap();
        crate::profile_restore::save_restore_state(
            &profile_path,
            &crate::profile_restore::ProfileRestoreState {
                schema_version: crate::profile_restore::PROFILE_RESTORE_SCHEMA_VERSION,
                affinity_records: Vec::new(),
                nice_records: vec![crate::profile_restore::NiceRestoreRecordV2 {
                    tid: 123,
                    process_pid: Some(123),
                    process_starttime_ticks: None,
                    task_starttime_ticks: None,
                    comm: Some("game".to_owned()),
                    original_nice: 0,
                    applied_nice: -5,
                }],
                ionice_records: Vec::new(),
            },
        )
        .unwrap();

        let outcome = run_restore_command_with_profile_paths(
            input::DaemonRestoreCommandInput {
                dry_run: true,
                emergency: true,
            },
            Some(journal_path),
            Some(audit_path),
            Some(history_path),
            affinity_path,
            profile_path,
        )
        .unwrap();

        assert_eq!(outcome.autotune.status, AutotuneRestoreStatus::Clean);
        assert!(outcome.profile.found_any());
        assert_eq!(outcome.profile.affinity_records, 1);
        assert_eq!(outcome.profile.profile_nice_records, 1);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn daemon_restore_summary_includes_autotune_and_profile_counts() {
        let autotune = AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::Restored,
            restored_actions: 1,
            failed_actions: 2,
            skipped_actions: 3,
            messages: Vec::new(),
        };
        let profile = restore::ProfileRestoreCommandOutcome {
            restored_any: true,
            summary: crate::profile_restore::ProfileRestoreSummary {
                affinity: 4,
                nice: 5,
                ionice: 6,
                skipped_dead: 7,
                skipped_identity_mismatch: 8,
                legacy_unverified: 9,
                errors: 10,
            },
            ..restore::ProfileRestoreCommandOutcome::default()
        };

        let summary = daemon_restore_summary_fields(&autotune, &profile);

        assert!(summary.contains("status=Restored"));
        assert!(summary.contains("restored_actions=1"));
        assert!(summary.contains("failed_actions=2"));
        assert!(summary.contains("skipped_actions=3"));
        assert!(summary.contains("profile_found=true"));
        assert!(summary.contains("profile_restored=15"));
        assert!(summary.contains("profile_skipped_total=15"));
        assert!(summary.contains("profile_skipped_dead=7"));
        assert!(summary.contains("profile_skipped_identity_mismatch=8"));
        assert!(summary.contains("profile_legacy_unverified=9"));
        assert!(summary.contains("profile_errors=10"));
    }

    #[test]
    fn daemon_status_text_contains_state_and_restore_command() {
        let output = DaemonStatusOutput {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_loaded: false,
            state: DaemonState::default(),
            capabilities: DaemonCapabilities {
                kernel_release: Some("6.9.1-test".to_owned()),
                btf_available: false,
                sched_tracepoints_available: true,
                perf_permissions_likely: true,
                perf_event_paranoid: Some(1),
                cgroup_v2_available: false,
                sched_ext_available: false,
                uclamp_available: false,
                ionice_available: true,
                irq_affinity_available: false,
                gpu_sysfs_available: false,
            },
            current_health: SystemHealthSnapshot::default(),
            watchdog: evaluate_daemon_watchdog(
                DaemonWatchdogInputs::from_state_and_health(
                    &DaemonState::default(),
                    &SystemHealthSnapshot::default(),
                ),
                &DaemonWatchdogConfig::default(),
            ),
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            recent_decisions: Vec::new(),
        };

        let text = render_status_text(&output);

        assert!(text.contains("Daemon status"));
        assert!(text.contains("state_loaded: false"));
        assert!(text.contains("phase: disabled"));
        assert!(text.contains("manual_restore_command: stutter daemon emergency-restore"));
        assert!(text.contains("active_workload: none"));
        assert!(text.contains("active_action: none"));
        assert!(text.contains("rollback_status: none"));
        assert!(text.contains("health: healthy"));
        assert!(text.contains("health_ok_for_apply: true"));
        assert!(text.contains("watchdog_ok: true"));
        assert!(text.contains("unavailable_features:"));
        assert!(text.contains("recent_decisions: none"));
    }

    #[test]
    fn daemon_explain_text_contains_why_and_change_sections() {
        let mut state = DaemonState {
            mode: crate::daemon::DaemonMode::Observe,
            phase: DaemonPhase::Paused,
            ..DaemonState::default()
        };
        state.degraded.push(crate::daemon::DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "insufficient samples".to_owned(),
        });
        let health = SystemHealthSnapshot::default();
        let watchdog = evaluate_daemon_watchdog(
            DaemonWatchdogInputs::from_state_and_health(&state, &health),
            &DaemonWatchdogConfig::default(),
        );
        let status = DaemonStatusOutput {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_loaded: true,
            capabilities: CapabilityProbe::default().probe(),
            state: state.clone(),
            current_health: health,
            watchdog,
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            recent_decisions: Vec::new(),
        };
        let policy = build_policy_from_daemon_state(&state);
        let output = DaemonExplainOutput {
            status_explanation: DaemonStatusExplanation::from_state_health_watchdog(
                &status.state,
                &status.current_health,
                &status.watchdog,
            ),
            policy_explanation: DaemonPolicyExplanation::from_policy(&policy),
            policy,
            status,
        };

        let text = render_explain_text(&output);

        assert!(text.contains("Daemon explain"));
        assert!(text.contains("Why no optimize"));
        assert!(text.contains("observe_only_mode"));
        assert!(text.contains("daemon_paused"));
        assert!(text.contains("insufficient samples"));
        assert!(text.contains("What changed"));
        assert!(text.contains("phase:paused"));
        assert!(text.contains("Policy decisions"));
    }

    #[test]
    fn daemon_why_and_what_changed_commands_render_focused_outputs() {
        let mut state = DaemonState {
            mode: crate::daemon::DaemonMode::Observe,
            phase: DaemonPhase::Paused,
            ..DaemonState::default()
        };
        state.active_target = Some(crate::daemon::DaemonTargetState {
            root_pid: Some(4242),
            active_targets: 3,
            comm: Some("Game.exe".to_owned()),
        });
        state.degraded.push(crate::daemon::DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "insufficient samples".to_owned(),
        });
        let health = SystemHealthSnapshot::default();
        let watchdog = evaluate_daemon_watchdog(
            DaemonWatchdogInputs::from_state_and_health(&state, &health),
            &DaemonWatchdogConfig::default(),
        );
        let status = DaemonStatusOutput {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_loaded: true,
            capabilities: CapabilityProbe::default().probe(),
            state: state.clone(),
            current_health: health,
            watchdog,
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            recent_decisions: Vec::new(),
        };
        let policy = build_policy_from_daemon_state(&state);
        let explain = DaemonExplainOutput {
            status_explanation: DaemonStatusExplanation::from_state_health_watchdog(
                &status.state,
                &status.current_health,
                &status.watchdog,
            ),
            policy_explanation: DaemonPolicyExplanation::from_policy(&policy),
            policy,
            status,
        };

        let why = why_not_optimize_output_from_explain(explain.clone());
        let what = what_changed_output_from_explain(explain);
        let why_text = render_why_not_optimize_text(&why);
        let what_text = render_what_changed_text(&what);

        assert!(why_text.contains("Daemon why-not-optimize"));
        assert!(why_text.contains("observe_only_mode"));
        assert!(why_text.contains("daemon_paused"));
        assert!(why_text.contains("insufficient samples"));
        assert!(what_text.contains("Daemon what-changed"));
        assert!(what_text.contains("phase:paused"));
        assert!(what_text.contains("active_workload:root_pid=4242"));
    }

    #[test]
    fn daemon_status_text_contains_active_workload_action_score_and_recent_decisions() {
        let state = DaemonState {
            mode: crate::daemon::DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Measure,
            active_target: Some(crate::daemon::DaemonTargetState {
                root_pid: Some(4242),
                active_targets: 3,
                comm: Some("Game.exe".to_owned()),
            }),
            active_experiment: Some(crate::daemon::DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity:game".to_owned(),
                candidate_name: Some("game-affinity".to_owned()),
                mode: crate::daemon::DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(1),
            }),
            active_rollback: Some(crate::daemon::DaemonRollbackState {
                action_id: "cpu-affinity:game".to_owned(),
                mode: crate::daemon::DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            }),
            last_decision: Some(crate::daemon::DaemonDecisionState {
                decision: "candidate_applied".to_owned(),
                reason: "measurement started".to_owned(),
                unix_nanos: Some(2),
                score_total: Some(42),
                candidate_count: None,
                top_denied_reason: None,
                planner: None,
                situation: None,
                focus_kind: None,
            }),
            ..DaemonState::default()
        };

        let output = DaemonStatusOutput {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_loaded: true,
            watchdog: evaluate_daemon_watchdog(
                DaemonWatchdogInputs::from_state_and_health(
                    &state,
                    &SystemHealthSnapshot::default(),
                ),
                &DaemonWatchdogConfig::default(),
            ),
            state,
            capabilities: CapabilityProbe::default().probe(),
            current_health: SystemHealthSnapshot::default(),
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            recent_decisions: vec![DaemonRecentDecision {
                unix_nanos: 3,
                phase: "Measuring".to_owned(),
                mode: "ApplyLowRisk".to_owned(),
                decision: "candidate_applied".to_owned(),
                candidate_name: Some("game-affinity".to_owned()),
                action_id: Some("cpu-affinity:game".to_owned()),
                rollback_performed: false,
                reason: "measurement started".to_owned(),
            }],
        };

        let text = render_status_text(&output);

        assert!(text.contains("active_workload: root_pid=4242 comm=Game.exe targets=3"));
        assert!(text.contains("active_action: action_id=cpu-affinity:game"));
        assert!(text.contains("active_action: action_id=cpu-affinity:game candidate=game-affinity mode=apply-low-risk safety_class=ReversibleLowRisk"));
        assert!(text.contains("rollback_status: action_id=cpu-affinity:game mode=apply-low-risk safety_class=ReversibleLowRisk available=true"));
        assert!(text.contains("current_score: 42"));
        assert!(text.contains("recent_decisions:"));
        assert!(text.contains("decision=candidate_applied"));
    }

    #[test]
    fn daemon_watch_line_is_compact_and_notification_only_tracks_meaningful_changes() {
        let mut output = build_status_output();
        output.state.phase = DaemonPhase::Observe;
        output.state.active_target = Some(crate::daemon::DaemonTargetState {
            root_pid: Some(1234),
            active_targets: 1,
            comm: Some("Game.exe".to_owned()),
        });

        let line = render_watch_line(&output);
        assert!(line.contains("phase=observe"));
        assert!(line.contains("workload=Game.exe:1234"));
        assert!(line.contains("restore=\"stutter daemon emergency-restore\""));

        let previous = DaemonWatchSignature::from_output(&output);
        output.state.active_experiment = Some(crate::daemon::DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "cpu-affinity:game".to_owned(),
            candidate_name: Some("game-affinity".to_owned()),
            mode: crate::daemon::DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(1),
        });
        let current = DaemonWatchSignature::from_output(&output);
        assert_eq!(
            render_watch_notification(&previous, &current).as_deref(),
            Some("action applied action_id=cpu-affinity:game")
        );
    }

    #[test]
    fn daemon_status_json_contains_state_capabilities_and_manual_restore() {
        let output = build_status_output();

        let json = render_status_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(value.get("state").is_some());
        assert!(value.get("capabilities").is_some());
        assert!(value.get("current_health").is_some());
        assert!(value.get("watchdog").is_some());
        assert_eq!(
            value["manual_restore_command"],
            "stutter daemon emergency-restore"
        );
    }

    #[test]
    fn daemon_bench_overhead_text_contains_budget_status() {
        let report = crate::daemon::evaluate_daemon_overhead(
            crate::daemon::DaemonOverheadSnapshot {
                sample_duration_millis: 1_000,
                cpu_millis_per_second: 1,
                memory_rss_bytes: Some(1024),
                open_fds: Some(8),
                disk_write_bytes_per_minute: Some(0),
            },
            crate::daemon::DaemonOverheadBudget::default(),
        );

        let text = render_bench_overhead_text(&report);

        assert!(text.contains("Daemon overhead benchmark"));
        assert!(text.contains("within_budget: true"));
        assert!(text.contains("cpu_millis_per_second:"));
    }

    #[test]
    fn daemon_soak_text_contains_budget_metrics() {
        let report = crate::daemon::run_fake_daemon_soak(&crate::daemon::DaemonSoakConfig {
            duration_seconds: 60,
            ..crate::daemon::DaemonSoakConfig::default()
        });

        let text = render_soak_text(&report);

        assert!(text.contains("Daemon scenario soak"));
        assert!(text.contains("passed: true"));
        assert!(text.contains("disk_growth_bytes:"));
        assert!(text.contains("event_drops: 0"));
    }

    #[test]
    fn daemon_acceptance_text_lists_final_boss_steps() {
        let report = crate::daemon::run_fake_daemon_acceptance_suite();

        let text = render_acceptance_text(&report);

        assert!(text.contains("Daemon acceptance"));
        assert!(text.contains("suite: fake-daemon-100-percent-acceptance"));
        assert!(text.contains("passed: true"));
        assert!(text.contains("step 1 install_service: passed"));
        assert!(text.contains("step 22 complete_audit_history: passed"));
    }

    #[test]
    fn daemon_doctor_text_reports_state_health_watchdog_and_checks() {
        let report = DaemonDoctorReport {
            state_path: "/tmp/daemon_state.json".to_owned(),
            state_load_ok: false,
            state_uncertain: true,
            safe_observe_only_required: true,
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            checks: vec![
                DaemonDoctorCheck {
                    name: "state_store_load".to_owned(),
                    passed: false,
                    message: "daemon state is missing or corrupt".to_owned(),
                },
                DaemonDoctorCheck {
                    name: "rollback_state".to_owned(),
                    passed: true,
                    message: "rollback state is clean".to_owned(),
                },
            ],
            capabilities: DaemonCapabilities {
                kernel_release: Some("6.9.1-test".to_owned()),
                btf_available: false,
                sched_tracepoints_available: true,
                perf_permissions_likely: true,
                perf_event_paranoid: Some(1),
                cgroup_v2_available: true,
                sched_ext_available: false,
                uclamp_available: true,
                ionice_available: true,
                irq_affinity_available: false,
                gpu_sysfs_available: false,
            },
            current_health: SystemHealthSnapshot::default(),
            watchdog: evaluate_daemon_watchdog(
                DaemonWatchdogInputs::from_state_and_health(
                    &DaemonState::default(),
                    &SystemHealthSnapshot::default(),
                ),
                &DaemonWatchdogConfig::default(),
            ),
        };

        let text = render_daemon_doctor_text(&report);

        assert!(text.contains("Daemon doctor"));
        assert!(text.contains("state_load_ok: false"));
        assert!(text.contains("state_uncertain: true"));
        assert!(text.contains("safe_observe_only_required: true"));
        assert!(text.contains("manual_restore_command: stutter daemon emergency-restore"));
        assert!(text.contains("health: healthy"));
        assert!(text.contains("watchdog_ok: true"));
        assert!(text.contains("kernel_release: 6.9.1-test"));
        assert!(text.contains("unavailable_features: btf, sched_ext, irq_affinity, gpu_sysfs"));
        assert!(text.contains("state_store_load: failed - daemon state is missing or corrupt"));
        assert!(text.contains("rollback_state: passed - rollback state is clean"));
    }

    #[test]
    fn daemon_reset_state_text_reports_dry_run_backup_and_safe_state() {
        let report = DaemonResetStateReport {
            state_path: "/tmp/daemon_state.json".to_owned(),
            dry_run: true,
            state_exists: true,
            backup_path: Some("/tmp/daemon_state.json.bak.1".to_owned()),
            reset_state: safe_reset_daemon_state(),
        };

        let text = render_reset_state_text(&report);

        assert!(text.contains("Daemon reset-state"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains("state_exists: true"));
        assert!(text.contains("backup_path: /tmp/daemon_state.json.bak.1"));
        assert!(text.contains("reset_mode: observe"));
        assert!(text.contains("reset_phase: disabled"));
        assert!(text.contains("reset_decision: daemon_state_reset"));
        assert!(text.contains("result: no changes written"));
    }

    #[test]
    fn safe_reset_daemon_state_clears_active_state_and_disables_apply() {
        let state = safe_reset_daemon_state();

        assert_eq!(state.mode, crate::daemon::DaemonMode::Observe);
        assert_eq!(state.phase, DaemonPhase::Disabled);
        assert!(state.active_target.is_none());
        assert!(state.active_experiment.is_none());
        assert!(state.active_rollback.is_none());
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("daemon_state_reset")
        );
    }
}
