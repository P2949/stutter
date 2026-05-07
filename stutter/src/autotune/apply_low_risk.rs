#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;

#[cfg(test)]
use crate::actions::{ActionState, ActionWarning};
use crate::{
    actions::{RollbackToken, SafetyClass, TuningAction, cpu_affinity::CpuAffinityProfileAction},
    autotune::candidate::{
        CandidateAction, CandidateDryRunRecord, dry_run_candidates,
        dry_run_record_from_action_state, generate_profile_candidates,
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

    run_apply_low_risk_candidate(plan.candidate, plan.duration).await
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
