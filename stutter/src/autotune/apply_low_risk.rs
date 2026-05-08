#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;

#[cfg(test)]
use crate::actions::{ActionId, ActionWarning};
use crate::{
    actions::{
        ActionState, RollbackToken, SafetyClass, TuningAction,
        cpu_affinity::CpuAffinityProfileAction,
        runner::{AuditedActionResult, run_audited_action, run_audited_action_with_audit_path},
    },
    autotune::{
        candidate::{
            CandidateAction, CandidateDryRunRecord, dry_run_candidates,
            dry_run_record_from_action_state, generate_profile_candidates,
        },
        washout::{WashoutWindowConfig, run_washout_for_action},
    },
    profiles::Profile,
};

#[derive(Clone, Debug)]
pub struct ApplyLowRiskPlan {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub candidate: CandidateAction,
    pub dry_run_record: CandidateDryRunRecord,
    pub duration: Duration,
}

#[derive(Clone, Debug)]
pub struct ApplyLowRiskOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub rollback_performed: bool,
}

pub trait LowRiskActionExecutor {
    fn candidate_name(&self) -> &str;
    fn action_kind(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply(&mut self) -> anyhow::Result<RollbackToken>;
    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()>;
}

pub struct CpuAffinityCandidateExecutor {
    candidate_name: String,
    action: CpuAffinityProfileAction,
}

impl CpuAffinityCandidateExecutor {
    pub fn from_candidate(candidate: CandidateAction) -> anyhow::Result<Self> {
        match candidate {
            CandidateAction::CpuAffinityProfile {
                profile_name,
                profile,
                tree_pid,
            } => Ok(Self {
                candidate_name: profile_name,
                action: CpuAffinityProfileAction {
                    tree_pid,
                    profile,
                    force_restore_overwrite: false,
                },
            }),
            #[cfg(test)]
            CandidateAction::Fake { .. } => {
                anyhow::bail!("apply-low-risk only supports CPU affinity profile actions")
            }
        }
    }
}

impl LowRiskActionExecutor for CpuAffinityCandidateExecutor {
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

pub struct AuditedRollbackGuard<'a, A: TuningAction + ?Sized> {
    action: &'a A,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

impl<'a, A: TuningAction + ?Sized> AuditedRollbackGuard<'a, A> {
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

impl<A: TuningAction + ?Sized> Drop for AuditedRollbackGuard<'_, A> {
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

struct RollbackGuard<'a, E: LowRiskActionExecutor + ?Sized> {
    executor: &'a mut E,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

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
        CandidateAction::CpuAffinityProfile {
            profile_name,
            profile,
            tree_pid,
        } => Ok((
            profile_name,
            CpuAffinityProfileAction {
                tree_pid,
                profile,
                force_restore_overwrite: false,
            },
        )),
        #[cfg(test)]
        CandidateAction::Fake { .. } => {
            anyhow::bail!("apply-low-risk only supports CPU affinity profile actions")
        }
    }
}

pub fn append_low_risk_history_event(
    path: &std::path::Path,
    event: &crate::autotune::history::AutotuneHistoryEvent,
) -> anyhow::Result<()> {
    crate::autotune::history::append_autotune_history_event(path, event)
}

pub fn register_audited_low_risk_outcome_for_exit_rollback(
    registry: &crate::autotune::shutdown::ActiveLowRiskActionRegistry,
    outcome: &AuditedCandidateApplyOutcome,
) {
    crate::autotune::shutdown::register_cpu_affinity_rollback(
        registry,
        format!("cpu-affinity-profile:{}", outcome.candidate_name),
        outcome.rollback.clone(),
    );
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
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action("autotune candidate", action, false).with_context(|| {
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

pub fn apply_cpu_affinity_candidate_with_audit_path_for_tests(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
    audit_path: &std::path::Path,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action_with_audit_path("autotune candidate", action, false, audit_path)
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

pub async fn run_apply_low_risk_candidate(
    candidate: CandidateAction,
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let mut executor = CpuAffinityCandidateExecutor::from_candidate(candidate)?;
    run_apply_low_risk_with_executor(&mut executor, duration).await
}

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
            "apply-low-risk only supports CPU affinity profile actions; blocked action_kind={}",
            action_kind
        );
    }

    if *safety_class != SafetyClass::ReversibleLowRisk {
        anyhow::bail!(
            "apply-low-risk only supports ReversibleLowRisk actions; blocked safety_class={:?}",
            safety_class
        );
    }

    Ok(())
}

pub fn resolve_one_target_tree_pid(
    tree_pid: Option<u32>,
    watch_process: Option<&str>,
) -> anyhow::Result<u32> {
    resolve_one_target_tree_pid_at(Path::new("/proc"), tree_pid, watch_process)
}

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

pub fn resolve_low_risk_experiment<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment(experiment, result, rollback_executor)
}

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

pub fn ensure_candidate_measurement_ready_for_decision(
    measurement_status: &crate::autotune::measurement::CandidateMeasurementWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    crate::autotune::measurement::ensure_candidate_measurement_ready_for_decision(
        measurement_status,
    )
}

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

pub async fn apply_low_risk_command(
    input: &crate::autotune::AutotuneCommandInput,
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

    crate::autotune::controller_journal::write_controller_journal_applying(
        &journal_path,
        &experiment_id,
        &action_id,
    )
    .with_context(|| {
        format!(
            "failed to write applying controller journal for autotune candidate '{}'",
            candidate_name
        )
    })?;

    let audited = apply_cpu_affinity_candidate_with_audit(candidate_name.clone(), &action)?;
    let _affected_tasks = audited.affected_tasks;
    let mut guard = AuditedRollbackGuard::new(&action, audited.rollback.clone());

    crate::autotune::controller_journal::write_controller_journal_applied(
        &journal_path,
        &experiment_id,
        &action_id,
        audited.rollback.clone(),
    )
    .with_context(|| {
        format!(
            "failed to write applied controller journal for autotune candidate '{}'",
            audited.candidate_name
        )
    })?;

    run_washout_for_action(&action, action.tree_pid, WashoutWindowConfig::default())
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

    crate::autotune::controller_journal::write_controller_journal_clean(&journal_path)
        .with_context(|| {
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
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeExecutor {
        candidate_name: String,
        action_kind: &'static str,
        safety_class: SafetyClass,
        dry_run_record: Option<CandidateDryRunRecord>,
        apply_token: Option<RollbackToken>,
        dry_run_calls: usize,
        apply_calls: usize,
        rollback_calls: usize,
    }

    impl FakeExecutor {
        fn low_risk() -> Self {
            Self {
                candidate_name: "game-main".to_owned(),
                action_kind: "cpu_affinity_profile",
                safety_class: SafetyClass::ReversibleLowRisk,
                dry_run_record: Some(CandidateDryRunRecord {
                    candidate_name: "game-main".to_owned(),
                    affected_tasks: 31,
                    warnings: Vec::new(),
                    safety_class: SafetyClass::ReversibleLowRisk,
                    eligible: true,
                    reason: None,
                }),
                apply_token: Some(RollbackToken::CpuAffinityRestoreFile {
                    path: PathBuf::from("/tmp/stutter-test-restore.json"),
                    affected_tasks: 31,
                }),
                dry_run_calls: 0,
                apply_calls: 0,
                rollback_calls: 0,
            }
        }
    }

    impl LowRiskActionExecutor for FakeExecutor {
        fn candidate_name(&self) -> &str {
            &self.candidate_name
        }

        fn action_kind(&self) -> &'static str {
            self.action_kind
        }

        fn safety_class(&self) -> SafetyClass {
            self.safety_class.clone()
        }

        fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord> {
            self.dry_run_calls += 1;
            Ok(self
                .dry_run_record
                .clone()
                .expect("fake dry-run record must be configured"))
        }

        fn apply(&mut self) -> anyhow::Result<RollbackToken> {
            self.apply_calls += 1;
            Ok(self
                .apply_token
                .clone()
                .expect("fake rollback token must be configured"))
        }

        fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
            self.rollback_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn low_risk_resolution_keeps_improved_candidate_as_current_profile() {
        use crate::{
            actions::RollbackToken,
            affinity::CpuMask,
            autotune::{
                candidate::CandidateAction,
                comparison::ExperimentResult,
                experiment::{ActiveExperiment, ExperimentId, ExperimentPhase, WindowScore},
                kept::ActiveProfileState,
                resolution::{ExperimentResolution, ExperimentRollbackExecutor},
            },
            process_tree::TaskClass,
            profiles::{Profile, ProfileRule},
            scorer::StutterScore,
        };

        struct FakeRollback {
            calls: usize,
        }

        impl ExperimentRollbackExecutor for FakeRollback {
            fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
                self.calls += 1;
                Ok(())
            }
        }

        let profile = Profile {
            name: "game-main".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 1234);
        let baseline = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 1_000,
                ..StutterScore::default()
            },
        };
        let candidate_score = WindowScore {
            started_unix_nanos: 300,
            finished_unix_nanos: 400,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 875,
                ..StutterScore::default()
            },
        };
        let mut experiment = ActiveExperiment::new(
            ExperimentId::new("low-risk-test"),
            candidate,
            baseline,
            1_000,
        );
        experiment.mark_candidate_applied(
            1_100,
            RollbackToken::CpuAffinityRestoreFile {
                path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            },
        );
        experiment.set_candidate_score(candidate_score);

        let mut rollback = FakeRollback { calls: 0 };
        let mut active_profile_state = ActiveProfileState::default();
        let resolution = resolve_low_risk_experiment_with_active_profile_state(
            &mut experiment,
            &ExperimentResult::Improved {
                improvement_percent: 12.5,
            },
            &mut rollback,
            &mut active_profile_state,
            9_999,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Kept { .. }));
        assert_eq!(rollback.calls, 0);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(experiment.has_rollback());
        assert_eq!(
            active_profile_state.current_profile_name(),
            Some("game-main")
        );
        assert_eq!(
            active_profile_state
                .current_rollback()
                .map(|rollback| rollback.affected_tasks()),
            Some(31)
        );
    }

    #[test]
    fn low_risk_resolution_reverts_inconclusive_result() {
        use crate::{
            actions::RollbackToken,
            affinity::CpuMask,
            autotune::{
                candidate::CandidateAction,
                comparison::ExperimentResult,
                experiment::{ActiveExperiment, ExperimentId, ExperimentPhase, WindowScore},
                resolution::{ExperimentResolution, ExperimentRollbackExecutor},
            },
            process_tree::TaskClass,
            profiles::{Profile, ProfileRule},
            scorer::StutterScore,
        };

        struct FakeRollback {
            calls: usize,
        }

        impl ExperimentRollbackExecutor for FakeRollback {
            fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
                self.calls += 1;
                Ok(())
            }
        }

        let profile = Profile {
            name: "game-main".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 1234);
        let score = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 1_000,
                ..StutterScore::default()
            },
        };
        let mut experiment =
            ActiveExperiment::new(ExperimentId::new("low-risk-test"), candidate, score, 1_000);
        experiment.mark_candidate_applied(
            1_100,
            RollbackToken::CpuAffinityRestoreFile {
                path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            },
        );

        let mut rollback = FakeRollback { calls: 0 };
        let resolution = resolve_low_risk_experiment(
            &mut experiment,
            &ExperimentResult::Inconclusive {
                reason: "not enough improvement".to_owned(),
            },
            &mut rollback,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Reverted { .. }));
        assert_eq!(rollback.calls, 1);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn low_risk_experiment_comparison_uses_conservative_thresholds() {
        let baseline = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 1_000,
                over_5ms: 10,
                frame_p99_ms: 12.0,
                frame_max_ms: 12.0,
                ..crate::scorer::StutterScore::default()
            },
        };
        let candidate = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 300,
            finished_unix_nanos: 400,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 875,
                over_5ms: 10,
                frame_p99_ms: 13.0,
                frame_max_ms: 13.0,
                ..crate::scorer::StutterScore::default()
            },
        };

        let result = compare_low_risk_experiment(
            &baseline,
            &candidate,
            crate::autotune::comparison::ExperimentDataQuality::High,
            false,
        );

        assert!(matches!(
            result,
            crate::autotune::comparison::ExperimentResult::Improved { .. }
        ));
    }

    #[test]
    fn candidate_measurement_not_ready_blocks_decision_gate() {
        let status = crate::autotune::measurement::CandidateMeasurementWindowStatus::Collecting {
            elapsed_ms: 10_000,
            scored_intervals: 5,
            scored_samples: 50,
            scored_task_count: 1,
            drop_counter_total: 0,
            reasons: vec!["candidate measurement window not complete".to_owned()],
        };

        let err = ensure_candidate_measurement_ready_for_decision(&status)
            .unwrap_err()
            .to_string();

        assert!(err.contains("candidate measurement window is not ready"));
        assert!(err.contains("candidate measurement window not complete"));
    }

    #[test]
    fn baseline_not_ready_blocks_apply_gate() {
        let status = crate::autotune::baseline::BaselineWindowStatus::Collecting {
            elapsed_ms: 10_000,
            scored_intervals: 5,
            scored_samples: 50,
            scored_task_count: 1,
            drop_counter_total: 0,
            reasons: vec!["baseline window not complete".to_owned()],
        };

        let err = ensure_baseline_ready_for_apply(&status)
            .unwrap_err()
            .to_string();

        assert!(err.contains("baseline window is not ready"));
        assert!(err.contains("baseline window not complete"));
    }

    struct TestAction {
        id: &'static str,
        safety_class: SafetyClass,
        should_fail_apply: bool,
        should_fail_verify: bool,
        affected_tasks: usize,
    }

    impl TuningAction for TestAction {
        fn id(&self) -> ActionId {
            ActionId(self.id.to_owned())
        }

        fn describe(&self) -> String {
            "test action".to_owned()
        }

        fn safety_class(&self) -> SafetyClass {
            self.safety_class.clone()
        }

        fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
            Ok(Vec::new())
        }

        fn dry_run(&self) -> anyhow::Result<ActionState> {
            Ok(ActionState {
                applied: false,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: self.affected_tasks,
                warnings: Vec::new(),
            })
        }

        fn apply(&self) -> anyhow::Result<RollbackToken> {
            if self.should_fail_apply {
                anyhow::bail!("intentional apply failure");
            }

            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-test-restore.json"),
                affected_tasks: self.affected_tasks,
            })
        }

        fn verify(&self) -> anyhow::Result<ActionState> {
            if self.should_fail_verify {
                anyhow::bail!("intentional verify failure");
            }

            Ok(ActionState {
                applied: true,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: 0,
                warnings: Vec::new(),
            })
        }

        fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn washout_config_defaults_are_safe_for_apply_low_risk() {
        let config = WashoutWindowConfig::default();

        assert_eq!(config.washout_seconds, 10);
        assert_eq!(config.verify_interval_ms, 1_000);
        assert_eq!(config.washout_ms(), 10_000);
    }

    #[test]
    fn low_risk_history_event_helper_writes_jsonl() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-low-risk-history-test-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");

        let score = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 143,
                ..crate::scorer::StutterScore::default()
            },
        };

        let event = crate::autotune::history::AutotuneHistoryEvent::new(
            "controller-1",
            crate::autotune::history::ControllerPhase::Cooldown,
            crate::autotune::history::AutotuneMode::ApplyLowRisk,
            None,
            crate::autotune::history::SituationKind::GameCpuSchedulerPressure,
            crate::autotune::history::observation_summary_from_window_score(
                true, 31, 0, "High", &score,
            ),
            crate::autotune::history::AutotuneDecisionSummary {
                decision: "Revert".to_owned(),
                candidate_name: Some("game-main".to_owned()),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            "regressed; rollback performed",
        )
        .with_rollback_performed(true);

        append_low_risk_history_event(&path, &event).unwrap();

        let events = crate::autotune::history::read_autotune_history_events(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
        assert!(events[0].rollback_performed);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_low_risk_outcome_registers_exit_rollback() {
        let registry = crate::autotune::shutdown::ActiveLowRiskActionRegistry::new();
        let outcome = AuditedCandidateApplyOutcome {
            candidate_name: "game-main".to_owned(),
            action_kind: "cpu_affinity_profile".to_owned(),
            affected_tasks: 31,
            safety_class: SafetyClass::ReversibleLowRisk,
            state: ActionState {
                applied: true,
                affected_tasks: 31,
                checked_tasks: 31,
                pending_changes: 0,
                warnings: Vec::new(),
            },
            rollback: RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            },
        };

        register_audited_low_risk_outcome_for_exit_rollback(&registry, &outcome);

        let actions = registry.snapshot();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "cpu-affinity-profile:game-main");
        assert_eq!(actions[0].action_kind, "cpu_affinity_profile");
        assert_eq!(actions[0].safety_class, SafetyClass::ReversibleLowRisk);
        assert_eq!(actions[0].rollback.affected_tasks(), 31);
    }

    #[test]
    fn audited_runner_logs_success_for_autotune_candidate() {
        let dir = temp_dir("audited-success");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: false,
            affected_tasks: 31,
        };

        let result =
            run_audited_action_with_audit_path("autotune candidate", &action, false, &audit_path)
                .unwrap();

        assert_eq!(result.state.affected_tasks, 31);
        assert!(result.rollback.is_some());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].command, "autotune candidate");
        assert_eq!(events[0].action_id.as_deref(), Some("test-candidate"));
        assert_eq!(events[0].safety_class, Some(SafetyClass::ReversibleLowRisk));
        assert!(!events[0].dry_run);
        assert!(events[0].success);
        assert_eq!(events[0].affected_tasks, 31);
        assert!(events[0].message.contains("action applied and verified"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_runner_logs_apply_failure_for_autotune_candidate() {
        let dir = temp_dir("audited-apply-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: true,
            should_fail_verify: false,
            affected_tasks: 31,
        };

        let result =
            run_audited_action_with_audit_path("autotune candidate", &action, false, &audit_path);

        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].command, "autotune candidate");
        assert!(!events[0].success);
        assert!(events[0].message.contains("apply failed"));
        assert!(events[0].message.contains("intentional apply failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_runner_logs_verify_failure_for_autotune_candidate() {
        let dir = temp_dir("audited-verify-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            id: "test-candidate",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: true,
            affected_tasks: 31,
        };

        let result =
            run_audited_action_with_audit_path("autotune candidate", &action, false, &audit_path);

        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].command, "autotune candidate");
        assert!(!events[0].success);
        assert!(events[0].message.contains("verify failed"));
        assert!(events[0].message.contains("intentional verify failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn apply_low_risk_applies_one_action_and_rolls_back() {
        let mut executor = FakeExecutor::low_risk();

        let outcome = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(outcome.candidate_name, "game-main");
        assert_eq!(outcome.action_kind, "cpu_affinity_profile");
        assert_eq!(outcome.affected_tasks, 31);
        assert_eq!(outcome.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(outcome.rollback_performed);
        assert_eq!(executor.dry_run_calls, 1);
        assert_eq!(executor.apply_calls, 1);
        assert_eq!(executor.rollback_calls, 1);
    }

    #[tokio::test]
    async fn high_risk_action_is_blocked_before_dry_run_or_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.safety_class = SafetyClass::HighRisk;

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("only supports ReversibleLowRisk"));
        assert_eq!(executor.dry_run_calls, 0);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[tokio::test]
    async fn non_cpu_affinity_action_is_blocked_before_dry_run_or_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.action_kind = "gpu_power_profile";

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("only supports CPU affinity profile actions"));
        assert_eq!(executor.dry_run_calls, 0);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[tokio::test]
    async fn zero_affected_tasks_are_blocked_before_apply() {
        let mut executor = FakeExecutor::low_risk();
        executor.dry_run_record = Some(CandidateDryRunRecord {
            candidate_name: "zero".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        });

        let err = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("not eligible"));
        assert_eq!(executor.dry_run_calls, 1);
        assert_eq!(executor.apply_calls, 0);
        assert_eq!(executor.rollback_calls, 0);
    }

    #[test]
    fn dry_run_warning_is_preserved_in_record() {
        let state = ActionState {
            applied: false,
            affected_tasks: 31,
            checked_tasks: 31,
            pending_changes: 31,
            warnings: vec![ActionWarning {
                message: "restore file already exists".to_owned(),
            }],
        };

        let record = dry_run_record_from_action_state(
            "warned".to_owned(),
            SafetyClass::ReversibleLowRisk,
            state,
        );

        assert!(record.eligible);
        assert_eq!(record.warnings.len(), 1);
        assert_eq!(record.warnings[0].message, "restore file already exists");
    }

    #[test]
    fn apply_low_risk_rejects_medium_risk_profile_action() {
        let err = ensure_low_risk_action_allowed(
            "cpu_affinity_profile",
            &SafetyClass::ReversibleMediumRisk,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("ReversibleLowRisk"));
        assert!(err.contains("ReversibleMediumRisk"));
    }

    #[test]
    fn target_selector_requires_exactly_one_selector() {
        let err = resolve_one_target_tree_pid_at(Path::new("/proc"), Some(1), Some("Game.exe"))
            .unwrap_err()
            .to_string();

        assert!(err.contains("exactly one target selector"));
    }

    #[test]
    fn watch_process_requires_exactly_one_match() {
        let dir = temp_dir("watch-process-many");
        fake_proc_with_comm(&dir, 10, "Game.exe");
        fake_proc_with_comm(&dir, 11, "Game.exe");

        let err = resolve_one_target_tree_pid_at(&dir, None, Some("Game.exe"))
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires one active target tree"));
        fs::remove_dir_all(dir).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-low-risk-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_proc_with_comm(proc_root: &Path, pid: u32, comm: &str) {
        let dir = proc_root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
    }
}
