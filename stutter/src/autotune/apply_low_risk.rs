#[cfg(test)]
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;

#[cfg(test)]
use crate::actions::runner::run_audited_action_with_audit_path;
#[cfg(test)]
use crate::actions::{ActionId, ActionWarning};
use crate::{
    actions::{
        ActionState, RollbackToken, SafetyClass, TuningAction,
        cpu_affinity::CpuAffinityProfileAction,
        runner::{
            ActionHooks, ActionRunPolicy, AuditedActionResult, run_audited_action_with_hooks,
        },
    },
    autotune::candidate::CandidateAction,
};
#[cfg(test)]
use crate::{
    autotune::{
        candidate::{
            CandidateDryRunRecord, dry_run_candidates, dry_run_record_from_action_state,
            generate_profile_candidates,
        },
        controller_journal::{
            ControllerJournalActionMetadata, journal_process_identity,
            write_controller_journal_applied_with_metadata,
            write_controller_journal_applying_with_metadata, write_controller_journal_clean,
        },
        washout::{WashoutWindowConfig, run_washout_for_action},
    },
    profiles::Profile,
};

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct ApplyLowRiskPlan {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub candidate: CandidateAction,
    pub dry_run_record: CandidateDryRunRecord,
    pub duration: Duration,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct ApplyLowRiskOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub rollback_performed: bool,
}

#[cfg(test)]
pub trait LowRiskActionExecutor {
    fn candidate_name(&self) -> &str;
    fn action_kind(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply(&mut self) -> anyhow::Result<RollbackToken>;
    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()>;
}

#[cfg(test)]
pub struct CpuAffinityLowRiskExecutor {
    candidate_name: String,
    action: CpuAffinityProfileAction,
}

#[cfg(test)]
impl CpuAffinityLowRiskExecutor {
    pub fn from_candidate(candidate: CandidateAction) -> anyhow::Result<Self> {
        match candidate {
            CandidateAction::CpuAffinityProfile { plan } => Ok(Self {
                candidate_name: plan.profile_name,
                action: CpuAffinityProfileAction {
                    tree_pid: plan.tree_pid,
                    profile: plan.profile,
                    force_restore_overwrite: false,
                },
            }),
            other => unsupported_low_risk_candidate(&other),
        }
    }
}

#[cfg(test)]
impl LowRiskActionExecutor for CpuAffinityLowRiskExecutor {
    fn candidate_name(&self) -> &str {
        &self.candidate_name
    }

    fn action_kind(&self) -> &'static str {
        "cpu_affinity_profile"
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord> {
        let safety_class = self.action.safety_class();
        let state = self.action.dry_run()?;
        Ok(dry_run_record_from_action_state(
            self.candidate_name.clone(),
            safety_class,
            state,
        ))
    }

    fn apply(&mut self) -> anyhow::Result<RollbackToken> {
        self.action.apply()
    }

    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

#[cfg(test)]
pub fn executor_for_low_risk_candidate(
    candidate: CandidateAction,
) -> anyhow::Result<Box<dyn LowRiskActionExecutor>> {
    Ok(Box::new(CpuAffinityLowRiskExecutor::from_candidate(
        candidate,
    )?))
}

fn unsupported_low_risk_candidate<T>(candidate: &CandidateAction) -> anyhow::Result<T> {
    anyhow::bail!(
        "apply-low-risk supports CPU-affinity profile actions only; candidate '{}' action_kind={} safety={:?} required_mode={}",
        candidate.candidate_name(),
        candidate.action_kind(),
        candidate.safety_class(),
        crate::daemon_policy::DaemonMode::ApplyMediumRisk
    )
}

#[cfg(test)]
pub struct AuditedRollbackGuard<'a, A: crate::actions::TuningAction + ?Sized> {
    action: &'a A,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

#[cfg(test)]
impl<'a, A: crate::actions::TuningAction + ?Sized> AuditedRollbackGuard<'a, A> {
    pub fn new(action: &'a A, token: RollbackToken) -> Self {
        Self {
            action,
            token: Some(token),
            rollback_performed: false,
        }
    }

    pub fn rollback_now(&mut self) -> anyhow::Result<()> {
        if let Some(token) = self.token.take() {
            self.action.rollback(&token)?;
            self.rollback_performed = true;
        }
        Ok(())
    }

    pub fn rollback_performed(&self) -> bool {
        self.rollback_performed
    }
}

#[cfg(test)]
impl<A: crate::actions::TuningAction + ?Sized> Drop for AuditedRollbackGuard<'_, A> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.action.rollback(&token) {
                log::error!("autotune_candidate_rollback_failed error={err:#}");
            } else {
                self.rollback_performed = true;
            }
        }
    }
}

#[cfg(test)]
struct RollbackGuard<'a, E: LowRiskActionExecutor + ?Sized> {
    executor: &'a mut E,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

#[cfg(test)]
impl<'a, E: LowRiskActionExecutor + ?Sized> RollbackGuard<'a, E> {
    fn new(executor: &'a mut E, token: RollbackToken) -> Self {
        Self {
            executor,
            token: Some(token),
            rollback_performed: false,
        }
    }

    fn rollback_now(&mut self) -> anyhow::Result<()> {
        if let Some(token) = self.token.take() {
            self.executor.rollback(&token)?;
            self.rollback_performed = true;
        }
        Ok(())
    }

    fn rollback_performed(&self) -> bool {
        self.rollback_performed
    }
}

#[cfg(test)]
impl<E: LowRiskActionExecutor + ?Sized> Drop for RollbackGuard<'_, E> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.executor.rollback(&token) {
                log::error!(
                    "autotune_apply_low_risk_rollback_failed candidate={} error={err:#}",
                    self.executor.candidate_name()
                );
            } else {
                self.rollback_performed = true;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuditedCandidateApplyOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub state: ActionState,
    pub rollback: RollbackToken,
}

pub fn action_from_candidate(
    candidate: CandidateAction,
) -> anyhow::Result<(String, CpuAffinityProfileAction)> {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => Ok((
            plan.profile_name,
            CpuAffinityProfileAction {
                tree_pid: plan.tree_pid,
                profile: plan.profile,
                force_restore_overwrite: false,
            },
        )),
        other => unsupported_low_risk_candidate(&other),
    }
}

#[cfg(test)]
pub fn append_low_risk_history_event(
    path: &std::path::Path,
    event: &crate::autotune::history::AutotuneHistoryEvent,
) -> anyhow::Result<()> {
    crate::autotune::history::append_autotune_history_event(path, event)
}

pub fn apply_candidate_with_audit(
    candidate: CandidateAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    let (candidate_name, action) = action_from_candidate(candidate)?;
    apply_cpu_affinity_candidate_with_audit(candidate_name, &action)
}

pub fn apply_cpu_affinity_candidate_with_audit(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    apply_cpu_affinity_candidate_with_audit_hooks(candidate_name, action, ActionHooks::none())
}

fn apply_cpu_affinity_candidate_with_audit_hooks(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
    hooks: ActionHooks<'_>,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let run_policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::AutotuneRuntime, false);
    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action_with_hooks("autotune candidate", action, run_policy, hooks)
        .with_context(|| {
            format!(
                "audited apply failed for autotune candidate '{}'",
                candidate_name
            )
        })?;

    let rollback = rollback.with_context(|| {
        format!(
            "audited apply for autotune candidate '{}' succeeded without rollback token",
            candidate_name
        )
    })?;

    Ok(AuditedCandidateApplyOutcome {
        candidate_name,
        action_kind: "cpu_affinity_profile".to_owned(),
        affected_tasks: rollback.affected_tasks(),
        safety_class: action.safety_class(),
        state,
        rollback,
    })
}

#[cfg(test)]
pub fn apply_cpu_affinity_candidate_with_audit_path_for_tests(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
    audit_path: &std::path::Path,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let run_policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test, false);
    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action_with_audit_path("autotune candidate", action, run_policy, audit_path)
        .with_context(|| {
            format!(
                "audited apply failed for autotune candidate '{}'",
                candidate_name
            )
        })?;

    let rollback = rollback.with_context(|| {
        format!(
            "audited apply for autotune candidate '{}' succeeded without rollback token",
            candidate_name
        )
    })?;

    Ok(AuditedCandidateApplyOutcome {
        candidate_name,
        action_kind: "cpu_affinity_profile".to_owned(),
        affected_tasks: rollback.affected_tasks(),
        safety_class: action.safety_class(),
        state,
        rollback,
    })
}

#[cfg(test)]
pub async fn run_apply_low_risk_candidate(
    candidate: CandidateAction,
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let mut executor = executor_for_low_risk_candidate(candidate)?;
    run_apply_low_risk_with_executor(executor.as_mut(), duration).await
}

#[cfg(test)]
pub async fn run_apply_low_risk_with_executor<E: LowRiskActionExecutor + ?Sized>(
    executor: &mut E,
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let candidate_name = executor.candidate_name().to_owned();
    let action_kind = executor.action_kind().to_owned();
    let safety_class = executor.safety_class();

    ensure_low_risk_action_allowed(&action_kind, &safety_class)?;

    let dry_run = executor.dry_run()?;
    if !dry_run.eligible {
        anyhow::bail!(
            "candidate '{}' is not eligible for apply-low-risk: {}",
            candidate_name,
            dry_run
                .reason
                .as_deref()
                .unwrap_or("dry-run did not produce an eligible candidate")
        );
    }

    let token = executor
        .apply()
        .with_context(|| format!("failed to apply low-risk candidate '{}'", candidate_name))?;
    let affected_tasks = token.affected_tasks();

    let mut guard = RollbackGuard::new(executor, token);

    if !duration.is_zero() {
        tokio::time::sleep(duration).await;
    }

    guard
        .rollback_now()
        .with_context(|| format!("failed to rollback low-risk candidate '{}'", candidate_name))?;

    Ok(ApplyLowRiskOutcome {
        candidate_name,
        action_kind,
        affected_tasks,
        safety_class,
        rollback_performed: guard.rollback_performed(),
    })
}

pub fn ensure_low_risk_action_allowed(
    action_kind: &str,
    safety_class: &SafetyClass,
) -> anyhow::Result<()> {
    if action_kind != "cpu_affinity_profile" {
        anyhow::bail!(
            "apply-low-risk currently supports CPU-affinity profile actions only; blocked action_kind={}",
            action_kind
        );
    }

    if *safety_class != SafetyClass::ReversibleLowRisk {
        anyhow::bail!(
            "apply-low-risk currently supports ReversibleLowRisk CPU-affinity profile actions only; blocked safety_class={:?}",
            safety_class
        );
    }

    Ok(())
}

#[cfg(test)]
pub fn resolve_one_target_tree_pid(
    tree_pid: Option<u32>,
    watch_process: Option<&str>,
) -> anyhow::Result<u32> {
    resolve_one_target_tree_pid_at(Path::new("/proc"), tree_pid, watch_process)
}

#[cfg(test)]
pub fn resolve_one_target_tree_pid_at(
    proc_root: &Path,
    tree_pid: Option<u32>,
    watch_process: Option<&str>,
) -> anyhow::Result<u32> {
    match (tree_pid, watch_process) {
        (Some(_), Some(_)) => {
            anyhow::bail!(
                "apply-low-risk requires exactly one target selector; pass either --tree-pid or --watch-process, not both"
            )
        }
        (None, None) => {
            anyhow::bail!(
                "apply-low-risk requires exactly one target selector; pass --tree-pid or --watch-process"
            )
        }
        (Some(pid), None) => {
            validate_one_live_tree_at(proc_root, pid)?;
            Ok(pid)
        }
        (None, Some(comm)) => {
            let matches = process_roots_by_comm_at(proc_root, comm)?;
            match matches.as_slice() {
                [pid] => {
                    validate_one_live_tree_at(proc_root, *pid)?;
                    Ok(*pid)
                }
                [] => anyhow::bail!(
                    "apply-low-risk could not find active target process with comm '{}'",
                    comm
                ),
                _ => anyhow::bail!(
                    "apply-low-risk requires one active target tree; comm '{}' matched {} processes: {:?}",
                    comm,
                    matches.len(),
                    matches
                ),
            }
        }
    }
}

#[cfg(test)]
fn validate_one_live_tree_at(proc_root: &Path, tree_pid: u32) -> anyhow::Result<()> {
    if tree_pid == 0 {
        anyhow::bail!("tree pid must be greater than zero");
    }

    let tree_pids = [tree_pid];
    let snapshot = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default()
            .proc_root(proc_root)
            .tree_pids(&tree_pids),
    );

    if !snapshot.process_roots.contains(&tree_pid) || snapshot.tasks.is_empty() {
        anyhow::bail!(
            "apply-low-risk target tree {} is not active or has no tasks",
            tree_pid
        );
    }

    Ok(())
}

#[cfg(test)]
fn process_roots_by_comm_at(proc_root: &Path, comm: &str) -> anyhow::Result<Vec<u32>> {
    let mut matches = Vec::new();

    for entry in fs::read_dir(proc_root)
        .with_context(|| format!("failed to read proc root {}", proc_root.display()))?
    {
        let entry = entry?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = file_name.parse::<u32>() else {
            continue;
        };

        let comm_path = entry.path().join("comm");
        let Ok(found_comm) = fs::read_to_string(comm_path) else {
            continue;
        };

        if found_comm.trim() == comm {
            matches.push(pid);
        }
    }

    matches.sort_unstable();
    Ok(matches)
}

#[cfg(test)]
pub fn select_first_eligible_low_risk_candidate(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
) -> anyhow::Result<CandidateAction> {
    for record in records {
        ensure_low_risk_action_allowed("cpu_affinity_profile", &record.safety_class)?;
        if !record.eligible {
            continue;
        }

        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.profile_name() == record.candidate_name)
        {
            return Ok(candidate.clone());
        }
    }

    anyhow::bail!("no eligible ReversibleLowRisk CPU affinity profile candidate found")
}

#[cfg(test)]
pub fn plan_apply_low_risk_from_profiles(
    tree_pid: u32,
    profiles_path: &Path,
    profiles: &[Profile],
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskPlan> {
    let candidates = generate_profile_candidates(profiles, tree_pid, None);
    if candidates.is_empty() {
        anyhow::bail!("no CPU affinity profile candidates were generated");
    }

    let records = dry_run_candidates(&candidates);
    let candidate = select_first_eligible_low_risk_candidate(&candidates, &records)?;
    let dry_run_record = records
        .into_iter()
        .find(|record| record.candidate_name == candidate.profile_name())
        .context("selected candidate did not have a matching dry-run record")?;

    Ok(ApplyLowRiskPlan {
        tree_pid,
        profiles_path: profiles_path.to_path_buf(),
        candidate,
        dry_run_record,
        duration,
    })
}

#[cfg(test)]
pub fn resolve_low_risk_experiment_with_active_profile_state<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
    active_profile_state: &mut crate::autotune::kept::ActiveProfileState,
    now_unix_nanos: u128,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment_with_active_profile_state(
        experiment,
        result,
        rollback_executor,
        active_profile_state,
        now_unix_nanos,
    )
}

#[cfg(test)]
pub fn resolve_low_risk_experiment<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment(experiment, result, rollback_executor)
}

#[cfg(test)]
pub fn compare_low_risk_experiment(
    baseline: &crate::autotune::experiment::WindowScore,
    candidate: &crate::autotune::experiment::WindowScore,
    data_quality: crate::autotune::comparison::ExperimentDataQuality,
    target_disappeared: bool,
) -> crate::autotune::comparison::ExperimentResult {
    crate::autotune::comparison::compare_experiment(
        crate::autotune::comparison::ExperimentComparisonInput {
            baseline,
            candidate,
            data_quality,
            target_disappeared,
        },
    )
}

#[cfg(test)]
pub fn ensure_candidate_measurement_ready_for_decision(
    measurement_status: &crate::autotune::measurement::CandidateMeasurementWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    crate::autotune::measurement::ensure_candidate_measurement_ready_for_decision(
        measurement_status,
    )
}

#[cfg(test)]
pub fn ensure_baseline_ready_for_apply(
    baseline_status: &crate::autotune::baseline::BaselineWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    match baseline_status {
        crate::autotune::baseline::BaselineWindowStatus::Ready { score } => Ok(score.clone()),
        crate::autotune::baseline::BaselineWindowStatus::Collecting { reasons, .. } => {
            anyhow::bail!(
                "baseline window is not ready; action blocked: {}",
                reasons.join("; ")
            )
        }
    }
}

#[cfg(test)]
fn controller_journal_metadata_for_cpu_affinity_action(
    candidate_name: &str,
    action: &CpuAffinityProfileAction,
    active_task_count: Option<usize>,
    verify_result: &'static str,
) -> ControllerJournalActionMetadata {
    let starttime_ticks =
        crate::process_tree::process_starttime_at(Path::new("/proc"), action.tree_pid);

    ControllerJournalActionMetadata::default()
        .with_candidate(candidate_name.to_owned())
        .with_workload_identity(journal_process_identity(
            action.tree_pid,
            starttime_ticks,
            None,
        ))
        .with_target_identity(journal_process_identity(
            action.tree_pid,
            starttime_ticks,
            active_task_count,
        ))
        .with_restore_command("stutter autotune restore")
        .with_verify_result(verify_result)
        .with_mode(crate::daemon_policy::DaemonMode::ApplyLowRisk)
        .with_safety_class(action.safety_class())
}

#[cfg(test)]
fn controller_journal_hooks_for_low_risk_action<'a>(
    journal_path: &'a Path,
    experiment_id: &'a str,
    action_id: &'a str,
    candidate_name: &'a str,
    action: &'a CpuAffinityProfileAction,
) -> ActionHooks<'a> {
    ActionHooks::after_apply(move |rollback| {
        write_controller_journal_applied_with_metadata(
            journal_path,
            experiment_id,
            action_id,
            rollback.clone(),
            controller_journal_metadata_for_cpu_affinity_action(
                candidate_name,
                action,
                Some(rollback.affected_tasks()),
                "applied_pending_verify",
            ),
        )
        .with_context(|| {
            format!(
                "failed to write applied controller journal for autotune candidate '{}'",
                candidate_name
            )
        })?;

        Ok(())
    })
    .with_after_rollback(move |_rollback| {
        crate::autotune::controller_journal::write_controller_journal_clean(journal_path)
            .with_context(|| {
                format!(
                    "failed to write clean controller journal after automatic rollback for autotune candidate '{}'",
                    candidate_name
                )
            })?;

        Ok(())
    })
}

#[cfg(test)]
pub async fn apply_low_risk_command(
    input: &crate::autotune::commands::live::AutotuneCommandInput,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let tree_pid = resolve_one_target_tree_pid(input.tree_pid, input.watch_process.as_deref())?;

    let profiles_path = input
        .profiles
        .as_deref()
        .context("apply-low-risk requires --profiles")?;

    let profiles = crate::profiles::load_profiles(profiles_path)?;
    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            profiles_path.display()
        );
    }

    let duration = Duration::from_secs(input.duration_seconds.unwrap_or(30));
    let plan = plan_apply_low_risk_from_profiles(tree_pid, profiles_path, &profiles, duration)?;

    let (candidate_name, action) = action_from_candidate(plan.candidate)?;
    let experiment_id = format!("apply-low-risk:{}", candidate_name);
    let action_id = format!("cpu-affinity-profile:{}", candidate_name);
    let journal_path = crate::autotune::controller_journal::default_controller_journal_path();

    write_controller_journal_applying_with_metadata(
        &journal_path,
        &experiment_id,
        &action_id,
        controller_journal_metadata_for_cpu_affinity_action(
            &candidate_name,
            &action,
            None,
            "pending_apply",
        ),
    )
    .with_context(|| {
        format!(
            "failed to write applying controller journal for autotune candidate '{}'",
            candidate_name
        )
    })?;

    let audited = apply_cpu_affinity_candidate_with_audit_hooks(
        candidate_name.clone(),
        &action,
        controller_journal_hooks_for_low_risk_action(
            &journal_path,
            &experiment_id,
            &action_id,
            &candidate_name,
            &action,
        ),
    )?;
    let _affected_tasks = audited.affected_tasks;
    let mut guard = AuditedRollbackGuard::new(&action, audited.rollback.clone());

    run_washout_for_action(
        &action,
        action.tree_pid,
        WashoutWindowConfig::default()
            .with_washout(input.washout_seconds, input.washout_verify_interval_ms),
    )
    .await
    .with_context(|| format!("washout failed for autotune candidate '{}'", candidate_name))?;

    if !plan.duration.is_zero() {
        tokio::time::sleep(plan.duration).await;
    }

    guard.rollback_now().with_context(|| {
        format!(
            "rollback failed for autotune candidate '{}'",
            audited.candidate_name
        )
    })?;

    write_controller_journal_clean(&journal_path).with_context(|| {
        format!(
            "failed to write clean controller journal after rolling back autotune candidate '{}'",
            audited.candidate_name
        )
    })?;

    Ok(ApplyLowRiskOutcome {
        candidate_name: audited.candidate_name,
        action_kind: audited.action_kind,
        affected_tasks: audited.affected_tasks,
        safety_class: audited.safety_class,
        rollback_performed: guard.rollback_performed(),
    })
}

#[cfg(test)]
#[path = "apply_low_risk_tests/mod.rs"]
mod tests;
