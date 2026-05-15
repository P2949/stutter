pub mod agent;
pub mod autotune;
pub mod daemon;
pub mod input;
pub mod misc;
pub mod monitor;
pub mod release;
pub mod report;
pub mod restore;
pub mod scenario;
pub mod service;

use crate::cli::AppCommand;

pub async fn dispatch(command: AppCommand) -> anyhow::Result<()> {
    match command {
        AppCommand::Monitor(input) => monitor::run_monitor_command(input).await,
        AppCommand::Bench(input) => monitor::run_bench_command(input).await,
        AppCommand::Version(input) => misc::run_version_command(input),
        AppCommand::Restore(input) => restore::run_restore_command(input.dry_run),
        AppCommand::ApplyProfile(input) => misc::run_apply_profile_command(input).await,
        AppCommand::InspectTree(input) => misc::run_inspect_tree_command(input),
        AppCommand::Summary(input) => report::run_summary_command(input),
        AppCommand::Validate(input) => report::run_validate_command(input),
        AppCommand::Report(input) => report::run_report_command(input),
        AppCommand::ReleaseCheck(input) => release::run_release_check_command(input),
        AppCommand::Tune(input) => misc::run_tune_command(input).await,
        AppCommand::Recommend(input) => report::run_recommend_command(input),
        AppCommand::Check(input) => report::run_check_command(input),
        AppCommand::ConfigCheck(input) => misc::run_config_check_command(input),
        AppCommand::ConfigExplain(input) => daemon::run_config_explain_command(input),
        AppCommand::AutotuneGenerateProfiles(input) => {
            autotune::run_generate_profiles_command(input)
        }
        AppCommand::AutotuneApplyCandidate(input) => autotune::run_apply_candidate_command(input),
        AppCommand::Autotune(input) => autotune::run_autotune_command(input).await,
        AppCommand::AutotuneStatus(input) => autotune::run_status_command(input),
        AppCommand::AutotuneReplayHistory(input) => autotune::run_replay_history_command(input),
        AppCommand::AutotuneRestore(input) => autotune::run_restore_command(input),
        AppCommand::Audit(input) => misc::run_audit_command(input),
        AppCommand::AutotuneReplay(input) => autotune::run_replay_command(input),
        AppCommand::Advisor(input) => misc::run_advisor_command(input).await,
        AppCommand::Doctor(input) => misc::run_doctor_command(input),
        AppCommand::Probes(input) => misc::run_probes_command(input),
        AppCommand::ProfileTemplate(input) => misc::run_profile_template_command(input),
        AppCommand::InspectIrqs(input) => misc::run_inspect_irqs_command(input),
        AppCommand::Agent(input) => agent::run_agent_command(input).await,
        AppCommand::DaemonConfigExplain(input) => daemon::run_config_explain_command(input),
        AppCommand::DaemonPolicyExplain(input) => daemon::run_policy_explain_command(input),
        AppCommand::DaemonProfiles(input) => daemon::run_profiles_command(input),
        AppCommand::DaemonExplain(input) => daemon::run_explain_command(input),
        AppCommand::DaemonWhyNotOptimize(input) => daemon::run_why_not_optimize_command(input),
        AppCommand::DaemonWhatChanged(input) => daemon::run_what_changed_command(input),
        AppCommand::DaemonStatus(input) => daemon::run_status_command(input),
        AppCommand::DaemonWatch(input) => daemon::run_watch_command(input),
        AppCommand::DaemonDoctor(input) => daemon::run_doctor_command(input),
        AppCommand::DaemonResetState(input) => daemon::run_reset_state_command(input),
        AppCommand::DaemonBenchOverhead(input) => daemon::run_bench_overhead_command(input),
        AppCommand::DaemonSoak(input) => daemon::run_soak_command(input),
        AppCommand::DaemonAcceptance(input) => daemon::run_acceptance_command(input),
        AppCommand::DaemonPause(input) => daemon::run_pause_command(input),
        AppCommand::DaemonResume(input) => daemon::run_resume_command(input),
        AppCommand::DaemonRestore(input) => daemon::run_restore_command(input),
        AppCommand::Completions(input) => misc::run_completions_command(input),
        AppCommand::Man(input) => misc::run_man_command(input),
        AppCommand::Rules(input) => misc::run_rules_command(input),
        AppCommand::ScenarioCreate(input) => scenario::run_create_command(input),
        AppCommand::ScenarioRun(input) => scenario::run_scenario_command(input).await,
        AppCommand::ScenarioCompare(input) => scenario::run_compare_command(input),
        AppCommand::ScenarioPath(input) => scenario::run_path_command(input),
        AppCommand::ScenarioList(input) => scenario::run_list_command(input),
        AppCommand::Service(input) => service::run_service_command(input),
    }
}
