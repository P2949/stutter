//! Runtime configuration construction and validation; this module owns config shape, not controller execution.

use std::path::PathBuf;

use crate::{
    autotune::{
        AutotuneRuntimeError,
        candidate::CandidateAction,
        history::default_autotune_history_path,
        quality::OnlineDataQualityPolicy,
        runtime::DEFAULT_RUNTIME_WINDOW_SECONDS,
        washout::WashoutWindowConfig,
        workload_policy::{DaemonWorkloadPolicyConfig, WorkloadPolicyMatrix},
    },
    daemon::{
        DaemonConfig, DaemonPolicy,
        policy::{ActionSource, DaemonMode, DaemonPolicyBuildInput, build_daemon_policy},
    },
    profiles::Profile,
};

#[derive(Clone, Debug)]
pub struct AutotuneRuntimeConfig {
    pub daemon_config: DaemonConfig,
    pub daemon_policy: DaemonPolicy,
    pub controller_id: String,
    pub decision_log: Option<PathBuf>,
    pub history_log: Option<PathBuf>,
    pub controller_journal_path: Option<PathBuf>,
    pub window_seconds: u64,
    pub candidate_window_seconds: u64,
    pub profiles: Vec<Profile>,
    pub online_data_quality_policy: OnlineDataQualityPolicy,
    pub workload_policy: WorkloadPolicyMatrix,
    pub workload_policy_error: Option<String>,
    pub washout: WashoutWindowConfig,
    pub dry_run_all_safe: bool,
    pub dry_run_plan_dir: Option<PathBuf>,
    pub simulated_candidates: Vec<CandidateAction>,
    pub simulate_action_effects: bool,
}

fn resolve_workload_policy_config(
    config: &DaemonWorkloadPolicyConfig,
) -> (WorkloadPolicyMatrix, Option<String>) {
    match config.resolved_matrix() {
        Ok(matrix) => (matrix, None),
        Err(err) => (
            WorkloadPolicyMatrix::default_rules(),
            Some(format!("{err:#}")),
        ),
    }
}

pub fn daemon_config_for_runtime_mode(
    mode: DaemonMode,
    source: ActionSource,
    tree_pid: Option<u32>,
    watch_process: Option<String>,
) -> DaemonConfig {
    let mut config = DaemonConfig {
        mode,
        source,
        ..DaemonConfig::default()
    };
    if let Some(tree_pid) = tree_pid {
        config.target.tree_pids.push(tree_pid);
    }
    config.target.watch_process = watch_process;
    config.target.require_explicit_target = mode.supports_apply();
    config.safety.min_confidence = crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE;
    config.autotune.candidate_window_seconds = DEFAULT_RUNTIME_WINDOW_SECONDS;
    config.autotune.washout_seconds = crate::autotune::washout::DEFAULT_WASHOUT_SECONDS;
    config
}

pub(crate) fn validate_runtime_config(
    config: &AutotuneRuntimeConfig,
) -> Result<(), AutotuneRuntimeError> {
    if config.window_seconds == 0 {
        return Err(AutotuneRuntimeError::InvalidMode {
            message: "window_seconds must be greater than zero".to_owned(),
        });
    }
    if config.candidate_window_seconds == 0 {
        return Err(AutotuneRuntimeError::InvalidMode {
            message: "candidate_window_seconds must be greater than zero".to_owned(),
        });
    }
    if config.dry_run_all_safe && config.mode() != DaemonMode::Suggest {
        return Err(AutotuneRuntimeError::InvalidMode {
            message: "--dry-run-all-safe requires suggest mode".to_owned(),
        });
    }
    Ok(())
}

impl AutotuneRuntimeConfig {
    pub fn observe(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::Observe,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn suggest(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn apply_low_risk(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::ApplyLowRisk,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn from_daemon_config(daemon_config: DaemonConfig, decision_log: Option<PathBuf>) -> Self {
        let daemon_policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &daemon_config,
            remote_context: None,
        });
        Self::from_daemon_parts(daemon_config, daemon_policy, decision_log)
    }

    pub fn from_daemon_parts(
        daemon_config: DaemonConfig,
        daemon_policy: DaemonPolicy,
        decision_log: Option<PathBuf>,
    ) -> Self {
        let candidate_window_seconds = daemon_config.autotune.candidate_window_seconds.max(1);
        let (workload_policy, workload_policy_error) =
            resolve_workload_policy_config(&daemon_config.autotune.workload_policy);

        Self {
            daemon_config,
            daemon_policy,
            controller_id: "local-autotune".to_owned(),
            decision_log,
            history_log: Some(default_autotune_history_path()),
            controller_journal_path: None,
            window_seconds: DEFAULT_RUNTIME_WINDOW_SECONDS,
            candidate_window_seconds,
            profiles: Vec::new(),
            online_data_quality_policy: OnlineDataQualityPolicy::default(),
            workload_policy,
            workload_policy_error,
            washout: WashoutWindowConfig::default(),
            dry_run_all_safe: false,
            dry_run_plan_dir: None,
            simulated_candidates: Vec::new(),
            simulate_action_effects: false,
        }
    }

    fn for_mode(
        mode: DaemonMode,
        source: ActionSource,
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        let daemon_config = daemon_config_for_runtime_mode(mode, source, tree_pid, watch_process);
        Self::from_daemon_config(daemon_config, decision_log)
    }

    pub fn with_profiles(mut self, profiles: Vec<Profile>) -> Self {
        self.profiles = profiles;
        self
    }

    pub fn with_candidate_window_seconds(mut self, seconds: u64) -> Self {
        let seconds = seconds.max(1);
        self.candidate_window_seconds = seconds;
        self.daemon_config.autotune.candidate_window_seconds = seconds;
        self
    }

    pub fn with_online_data_quality_policy(mut self, policy: OnlineDataQualityPolicy) -> Self {
        self.online_data_quality_policy = policy;
        self
    }

    pub fn with_min_focus_confidence(mut self, value: f32) -> Self {
        self.daemon_config.safety.min_confidence = value.clamp(0.0, 1.0);
        self.refresh_daemon_policy();
        self
    }

    pub fn with_washout(mut self, seconds: u64, verify_interval_ms: u64) -> Self {
        self.washout = WashoutWindowConfig::default().with_washout(seconds, verify_interval_ms);
        self.daemon_config.autotune.washout_seconds = seconds;
        self
    }

    pub fn with_dry_run_all_safe(mut self, enabled: bool) -> Self {
        self.dry_run_all_safe = enabled;
        self
    }

    pub fn with_dry_run_plan_dir(mut self, path: PathBuf) -> Self {
        self.dry_run_plan_dir = Some(path);
        self
    }

    pub fn with_simulated_candidates(mut self, candidates: Vec<CandidateAction>) -> Self {
        self.simulated_candidates = candidates;
        self
    }

    pub fn with_simulated_action_effects(mut self) -> Self {
        self.simulate_action_effects = true;
        self
    }

    pub fn mode(&self) -> DaemonMode {
        self.daemon_config.mode
    }

    pub fn tree_pid(&self) -> Option<u32> {
        self.daemon_config.target.tree_pids.first().copied()
    }

    pub fn watch_process(&self) -> Option<&str> {
        self.daemon_config.target.watch_process.as_deref()
    }

    fn refresh_daemon_policy(&mut self) {
        self.daemon_policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &self.daemon_config,
            remote_context: None,
        });
    }
}
