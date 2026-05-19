use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use crate::{
    autotune::commands::live::AutotuneCommandInput as LiveAutotuneCommandInput,
    config::model::MonitorConfig,
    daemon::testing::DaemonSoakConfig,
    doctor::DoctorInput,
    process_tree::TaskClass,
    release::{ReleaseChannel, ReleaseReadinessInputs},
    service::ServiceCommandRequest,
};

#[derive(Debug)]
pub enum AppCommand {
    Monitor(MonitorCommandInput),
    Bench(BenchCommandInput),
    Version(VersionCommandInput),
    Restore(RestoreCommandInput),
    ApplyProfile(ApplyProfileCommandInput),
    InspectTree(InspectTreeCommandInput),
    Summary(SummaryCommandInput),
    Validate(ValidateCommandInput),
    Report(ReportCommandInput),
    ReleaseCheck(ReleaseCheckCommandInput),
    Tune(TuneCommandInput),
    Recommend(RecommendCommandInput),
    Check(CheckCommandInput),
    DisplayPathCompare(DisplayPathCompareCommandInput),
    ConfigCheck(ConfigCheckCommandInput),
    ConfigExplain(DaemonConfigExplainCommandInput),
    AutotuneGenerateProfiles(AutotuneGenerateProfilesCommandInput),
    AutotuneApplyCandidate(AutotuneApplyCandidateCommandInput),
    Autotune(AutotuneCommandInput),
    AutotuneStatus(AutotuneStatusCommandInput),
    AutotuneReplayHistory(AutotuneReplayHistoryCommandInput),
    AutotuneRestore(AutotuneRestoreCommandInput),
    Audit(AuditCommandInput),
    AutotuneReplay(AutotuneReplayCommandInput),
    Advisor(AdvisorCommandInput),
    Doctor(DoctorCommandInput),
    Probes(ProbesCommandInput),
    ProfileTemplate(ProfileTemplateCommandInput),
    InspectIrqs(InspectIrqsCommandInput),
    InspectDrmTracepoints(InspectDrmTracepointsCommandInput),
    WaylandProbe(WaylandProbeCommandInput),
    Agent(AgentCommandInput),
    PrivilegedWorker(PrivilegedWorkerCommandInput),
    DaemonConfigExplain(DaemonConfigExplainCommandInput),
    DaemonPolicyExplain(DaemonPolicyExplainCommandInput),
    DaemonPolicyLint(DaemonPolicyLintCommandInput),
    DaemonProfiles(DaemonProfilesCommandInput),
    DaemonExplain(DaemonExplainCommandInput),
    DaemonWhyNotOptimize(DaemonWhyNotOptimizeCommandInput),
    DaemonWhatChanged(DaemonWhatChangedCommandInput),
    DaemonStatus(DaemonStatusCommandInput),
    DaemonWatch(DaemonWatchCommandInput),
    DaemonDoctor(DaemonDoctorCommandInput),
    DaemonResetState(DaemonResetStateCommandInput),
    DaemonBenchOverhead(DaemonBenchOverheadCommandInput),
    DaemonSoak(DaemonSoakCommandInput),
    DaemonAcceptance(DaemonAcceptanceCommandInput),
    DaemonPause(DaemonPauseCommandInput),
    DaemonResume(DaemonResumeCommandInput),
    DaemonResyncState(DaemonResyncStateCommandInput),
    DaemonRestore(DaemonRestoreCommandInput),
    Completions(CompletionsCommandInput),
    Man(ManCommandInput),
    Rules(RulesCommandInput),
    Scenario(ScenarioCommandInput),
    Service(ServiceCommandInput),
}

#[derive(Debug)]
pub struct MonitorCommandInput {
    pub config: Arc<MonitorConfig>,
}

#[derive(Debug)]
pub struct BenchCommandInput {
    pub config: Arc<MonitorConfig>,
    pub role: String,
    pub run_name: String,
}

#[derive(Debug)]
pub struct RestoreCommandInput {
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct VersionCommandInput {
    pub features: bool,
}

#[derive(Debug)]
pub struct ApplyProfileCommandInput {
    pub tree_pid: u32,
    pub profile: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    pub allow_medium_risk: bool,
    pub watch: bool,
    pub keep_applied: bool,
    pub refresh_ms: u64,
    pub enforce: bool,
}

#[derive(Debug)]
pub struct InspectTreeCommandInput {
    pub tree_pid: u32,
}

#[derive(Debug)]
pub struct SummaryCommandInput {
    pub path: PathBuf,
    pub json: bool,
    pub top: usize,
    pub filter_class: Option<TaskClass>,
}

#[derive(Debug)]
pub struct ValidateCommandInput {
    pub path: PathBuf,
    pub json: bool,
    pub strict: bool,
}

#[derive(Debug)]
pub struct ReportCommandInput {
    pub path: Option<PathBuf>,
    pub json: bool,
    pub analysis_json: bool,
    pub json_summary: bool,
    pub html: Option<PathBuf>,
    pub top: usize,
    pub cluster_window_ms: u64,
    pub batch: Option<PathBuf>,
    pub diff: Option<PathBuf>,
    pub filter_class: Option<TaskClass>,
    pub flamegraph: Option<PathBuf>,
}

#[derive(Debug)]
pub struct DisplayPathCompareCommandInput {
    pub baseline: PathBuf,
    pub test: PathBuf,
    pub json: bool,
}

#[derive(Debug)]
pub struct TuneCommandInput {
    pub tree_pid: u32,
    pub profiles: PathBuf,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub runs: u32,
    pub keep_best: bool,
    pub baseline_profile: Option<String>,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log: Option<PathBuf>,
    pub enforce: bool,
    pub hwmon: bool,
}

#[derive(Debug)]
pub struct RecommendCommandInput {
    pub baseline: PathBuf,
    pub tune: PathBuf,
    pub json: bool,
    pub markdown: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ReleaseCheckCommandInput {
    pub channel: ReleaseChannel,
    pub inputs: ReleaseReadinessInputs,
    pub json: bool,
    pub enforce: bool,
}

#[derive(Debug)]
pub struct CheckCommandInput {
    pub baseline: PathBuf,
    pub current: PathBuf,
    pub max_regression_p99_ms: Option<f64>,
    pub max_max_regression_ms: Option<f64>,
    pub json: bool,
    pub top: usize,
    pub filter_class: Option<TaskClass>,
}

#[derive(Debug)]
pub struct ConfigCheckCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct AutotuneGenerateProfilesCommandInput {
    pub watch_process: Option<String>,
    pub out: PathBuf,
    pub allow_cpus: Option<String>,
    pub deny_cpus: Option<String>,
    pub min_render_cpus: usize,
    pub min_game_cpus: usize,
    pub min_compositor_cpus: usize,
    pub min_background_cpus: usize,
}

#[derive(Debug)]
pub struct AutotuneCommandInput {
    pub input: LiveAutotuneCommandInput,
}

#[derive(Debug)]
pub struct AutotuneStatusCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct PrivilegedWorkerCommandInput {
    pub socket: PathBuf,
}

#[derive(Debug)]
pub struct AutotuneReplayHistoryCommandInput {
    pub history: PathBuf,
}

#[derive(Debug)]
pub struct AutotuneRestoreCommandInput {
    pub journal: Option<PathBuf>,
    pub audit: Option<PathBuf>,
    pub history: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct AutotuneApplyCandidateCommandInput {
    pub candidate_json: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct AutotuneReplayCommandInput {
    pub run: PathBuf,
    pub config: Option<PathBuf>,
}

#[derive(Debug)]
pub struct AuditCommandInput {
    pub path: Option<PathBuf>,
    pub tail: usize,
    pub json: bool,
}

#[derive(Debug)]
pub struct AdvisorCommandInput {
    pub run: Option<PathBuf>,
    pub profiles: Option<PathBuf>,
    pub json: bool,
    pub watch_runs: bool,
    pub runs_dir: Option<PathBuf>,
    pub poll_seconds: u64,
    pub once: bool,
}

#[derive(Debug)]
pub struct DoctorCommandInput {
    pub input: DoctorInput,
}

#[derive(Debug)]
pub struct ProbesCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct ProfileTemplateCommandInput {
    pub topology: bool,
}

#[derive(Debug)]
pub struct InspectIrqsCommandInput {
    pub json: bool,
    pub filter: Vec<String>,
    pub top: usize,
}

#[derive(Debug)]
pub struct InspectDrmTracepointsCommandInput {
    pub json: bool,
    pub events_root: Option<PathBuf>,
}

#[derive(Debug)]
pub struct WaylandProbeCommandInput {
    pub duration: Duration,
    pub output: Option<String>,
    pub fullscreen: bool,
    pub out_dir: PathBuf,
}

#[derive(Debug)]
pub struct DaemonConfigExplainCommandInput {
    pub json: bool,
    pub preset: Option<String>,
}

#[derive(Debug)]
pub struct DaemonPolicyExplainCommandInput {
    pub json: bool,
    pub preset: Option<String>,
}

#[derive(Debug)]
pub struct DaemonPolicyLintCommandInput {
    pub json: bool,
    pub preset: Option<String>,
}

#[derive(Debug)]
pub enum DaemonProfilesCommandInput {
    List(DaemonProfilesListCommandInput),
    Forget(DaemonProfilesForgetCommandInput),
    Explain(DaemonProfilesExplainCommandInput),
}

#[derive(Debug)]
pub struct DaemonProfilesListCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonProfilesForgetCommandInput {
    pub workload_identity_hash: Option<String>,
    pub candidate: Option<String>,
    pub all: bool,
    pub dry_run: bool,
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonProfilesExplainCommandInput {
    pub workload_identity_hash: Option<String>,
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonExplainCommandInput {
    pub json: bool,
    pub explain_last: usize,
}

#[derive(Debug)]
pub struct DaemonWhyNotOptimizeCommandInput {
    pub json: bool,
    pub explain_last: usize,
}

#[derive(Debug)]
pub struct DaemonWhatChangedCommandInput {
    pub json: bool,
    pub explain_last: usize,
}

#[derive(Debug)]
pub struct DaemonStatusCommandInput {
    pub json: bool,
    pub explain_last: usize,
}

#[derive(Debug)]
pub struct DaemonWatchCommandInput {
    pub interval_ms: u64,
    pub iterations: Option<u64>,
    pub verbose: bool,
    pub explain_last: usize,
}

#[derive(Debug)]
pub struct DaemonDoctorCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonResetStateCommandInput {
    pub dry_run: bool,
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonBenchOverheadCommandInput {
    pub json: bool,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct DaemonSoakCommandInput {
    pub config: DaemonSoakConfig,
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonAcceptanceCommandInput {
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonPauseCommandInput;

#[derive(Debug)]
pub struct DaemonResumeCommandInput;

#[derive(Debug)]
pub struct DaemonResyncStateCommandInput {
    pub dry_run: bool,
    pub json: bool,
}

#[derive(Debug)]
pub struct DaemonRestoreCommandInput {
    pub dry_run: bool,
    pub emergency: bool,
}

#[derive(Debug)]
pub struct ServiceCommandInput {
    pub request: ServiceCommandRequest,
    pub json: bool,
}

#[derive(Debug)]
pub struct AgentCommandInput {
    pub bind: SocketAddr,
    pub unix_socket: Option<PathBuf>,
    pub runs_dir: Option<PathBuf>,
    pub allow_unsafe_bind: bool,
    pub bearer_token_env: String,
    pub bearer_token_file: Option<PathBuf>,
    pub read_token_env: String,
    pub read_token_file: Option<PathBuf>,
    pub apply_token_env: String,
    pub apply_token_file: Option<PathBuf>,
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
}

#[derive(Debug)]
pub struct CompletionsCommandInput {
    pub shell: clap_complete::Shell,
}

#[derive(Debug)]
pub struct ManCommandInput {
    pub output: Option<PathBuf>,
}

#[derive(Debug)]
pub enum RulesCommand {
    Import(RulesImportCommandInput),
    Check(RulesCheckArgs),
    List,
    Status,
    Enable(RulesEnableArgs),
    Disable,
    Remove(RulesRemoveArgs),
}

#[derive(Debug)]
pub struct RulesImportCommandInput {
    pub source: PathBuf,
    pub name: String,
    pub source_repo: Option<String>,
    pub source_commit: Option<String>,
    pub license: String,
    pub out: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct RulesCheckArgs {
    pub source: Option<PathBuf>,
    pub generated: Option<PathBuf>,
}

#[derive(Debug)]
pub struct RulesEnableArgs {
    pub name: String,
}

#[derive(Debug)]
pub struct RulesRemoveArgs {
    pub name: String,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct RulesCommandInput {
    pub command: RulesCommand,
}

#[derive(Debug)]
pub struct ScenarioCreateCommandInput {
    pub name: String,
    pub force: bool,
    pub watch_process: Option<String>,
    pub duration: u64,
    pub preset: String,
    pub mangohud_log: Option<PathBuf>,
    pub notes: Option<String>,
}

#[derive(Debug)]
pub struct ScenarioRunCommandInput {
    pub name: String,
    pub role: String,
    pub dry_run: bool,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log_override: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ScenarioCompareCommandInput {
    pub name: String,
    pub baseline: Option<PathBuf>,
    pub current: Option<PathBuf>,
    pub top: usize,
    pub json_summary: bool,
    pub validate: bool,
}

#[derive(Debug)]
pub struct ScenarioPathCommandInput {
    pub name: String,
}

#[derive(Debug)]
pub enum ScenarioCommand {
    Create(ScenarioCreateCommandInput),
    Run(ScenarioRunCommandInput),
    Compare(ScenarioCompareCommandInput),
    Path(ScenarioPathCommandInput),
    List,
}

#[derive(Debug)]
pub struct ScenarioListCommandInput;

#[derive(Debug)]
pub struct ScenarioCommandInput {
    pub command: ScenarioCommand,
}

#[cfg(test)]
mod runtime_decoupling_tests {
    use std::sync::Arc;

    use crate::config::model::MonitorConfig;

    #[test]
    fn monitor_command_inputs_store_resolved_monitor_config() {
        fn assert_arc_monitor_config(_: &Arc<MonitorConfig>) {}

        let monitor = super::MonitorCommandInput {
            config: Arc::new(MonitorConfig::default()),
        };
        assert_arc_monitor_config(&monitor.config);

        let bench = super::BenchCommandInput {
            config: monitor.config.clone(),
            role: "test-role".to_owned(),
            run_name: "test-run".to_owned(),
        };
        assert_arc_monitor_config(&bench.config);
    }

    #[test]
    fn long_running_runtime_sources_do_not_store_cli_config() {
        let forbidden_terms = [
            concat!("cli", "::", "Config"),
            concat!("crate", "::", "cli", "::", "Config"),
            concat!("from", "_existing", "_cli", "_config"),
            concat!("from", "_cli", "_config"),
        ];
        let sources = [
            ("stutter/src/commands/input.rs", include_str!("input.rs")),
            ("stutter/src/session.rs", include_str!("../session.rs")),
            (
                "stutter/src/probe_activation.rs",
                include_str!("../probe_activation.rs"),
            ),
            ("stutter/src/tasks.rs", include_str!("../tasks.rs")),
            (
                "stutter/src/ebpf_loader.rs",
                include_str!("../ebpf_loader.rs"),
            ),
            ("stutter/src/remote.rs", include_str!("../remote.rs")),
            ("stutter/src/agent.rs", include_str!("../agent.rs")),
            (
                "stutter/src/config/layer.rs",
                include_str!("../config/layer.rs"),
            ),
            (
                "stutter/src/config/effective.rs",
                include_str!("../config/effective.rs"),
            ),
            (
                "stutter/src/config/merge.rs",
                include_str!("../config/merge.rs"),
            ),
        ];

        for (path, source) in sources {
            for term in &forbidden_terms {
                assert!(
                    !source.contains(term),
                    "{path} still contains forbidden runtime coupling marker {term:?}"
                );
            }
        }
    }
}
