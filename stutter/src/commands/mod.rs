pub mod agent;
pub mod autotune;
pub mod misc;
pub mod monitor;
pub mod report;
pub mod restore;
pub mod scenario;

use crate::cli::AppCommand;

pub async fn dispatch(command: AppCommand) -> anyhow::Result<()> {
    match command {
        AppCommand::Monitor(config) => monitor::run_monitor_command(config).await,
        AppCommand::Bench {
            config,
            role,
            run_name,
        } => monitor::run_bench_command(config, role, run_name).await,
        AppCommand::Restore { dry_run } => restore::run_restore_command(dry_run),
        AppCommand::ApplyProfile {
            tree_pid,
            profile,
            force,
            dry_run,
            allow_medium_risk,
            watch,
            keep_applied,
            refresh_ms,
            enforce,
        } => {
            misc::run_apply_profile_command(
                tree_pid,
                profile,
                force,
                dry_run,
                allow_medium_risk,
                watch,
                keep_applied,
                refresh_ms,
                enforce,
            )
            .await
        }
        AppCommand::InspectTree { tree_pid } => misc::run_inspect_tree_command(tree_pid),
        AppCommand::Summary {
            path,
            json,
            top,
            filter_class,
        } => report::run_summary_command(path, json, top, filter_class),
        AppCommand::Validate { path, json, strict } => {
            report::run_validate_command(path, json, strict)
        }
        AppCommand::Report {
            path,
            json,
            analysis_json,
            json_summary,
            html,
            top,
            cluster_window_ms,
            batch,
            diff,
            filter_class,
            flamegraph,
        } => report::run_report_command(
            path,
            json,
            analysis_json,
            json_summary,
            html,
            top,
            cluster_window_ms,
            batch,
            diff,
            filter_class,
            flamegraph,
        ),
        AppCommand::Tune {
            tree_pid,
            profiles,
            epoch_seconds,
            warmup_seconds,
            runs,
            keep_best,
            baseline_profile,
            out_dir,
            mangohud_log,
            enforce,
            hwmon,
        } => {
            misc::run_tune_command(
                tree_pid,
                profiles,
                epoch_seconds,
                warmup_seconds,
                runs,
                keep_best,
                baseline_profile,
                out_dir,
                mangohud_log,
                enforce,
                hwmon,
            )
            .await
        }
        AppCommand::Recommend {
            baseline,
            tune,
            json,
            markdown,
        } => report::run_recommend_command(baseline, tune, json, markdown),
        AppCommand::Check {
            baseline,
            current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        } => report::run_check_command(
            baseline,
            current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        ),
        AppCommand::AutotuneGenerateProfiles {
            watch_process,
            out,
            allow_cpus,
            deny_cpus,
            min_render_cpus,
            min_game_cpus,
            min_compositor_cpus,
            min_background_cpus,
        } => autotune::run_generate_profiles_command(
            crate::autotune::generate_profiles::GenerateProfilesCommandInput {
                watch_process,
                out,
                allow_cpus,
                deny_cpus,
                min_render_cpus,
                min_game_cpus,
                min_compositor_cpus,
                min_background_cpus,
            },
        ),
        AppCommand::Autotune { input } => autotune::run_autotune_command(input).await,
        AppCommand::AutotuneStatus { json } => autotune::run_status_command(json),
        AppCommand::AutotuneReplayHistory { history } => {
            autotune::run_replay_history_command(history)
        }
        AppCommand::AutotuneRestore {
            journal,
            audit,
            history,
            dry_run,
        } => autotune::run_restore_command(journal, audit, history, dry_run),
        AppCommand::Audit { path, tail, json } => misc::run_audit_command(path, tail, json),
        AppCommand::AutotuneReplay { run, config } => autotune::run_replay_command(run, config),
        AppCommand::Advisor {
            run,
            profiles,
            json,
            watch_runs,
            runs_dir,
            poll_seconds,
            once,
        } => {
            misc::run_advisor_command(
                run,
                profiles,
                json,
                watch_runs,
                runs_dir,
                poll_seconds,
                once,
            )
            .await
        }
        AppCommand::Doctor { input } => misc::run_doctor_command(input),
        AppCommand::Probes { json } => misc::run_probes_command(json),
        AppCommand::ProfileTemplate { topology } => misc::run_profile_template_command(topology),
        AppCommand::InspectIrqs { json, filter, top } => {
            misc::run_inspect_irqs_command(json, filter, top)
        }
        AppCommand::Agent {
            bind,
            runs_dir,
            allow_unsafe_bind,
            bearer_token_env,
            bearer_token_file,
            max_duration_seconds,
            max_targets,
            max_concurrent_recordings,
        } => {
            agent::run_agent_command(
                bind,
                runs_dir,
                allow_unsafe_bind,
                bearer_token_env,
                bearer_token_file,
                max_duration_seconds,
                max_targets,
                max_concurrent_recordings,
            )
            .await
        }
        AppCommand::Completions { shell } => misc::run_completions_command(shell),
        AppCommand::Man { output } => misc::run_man_command(output),
        AppCommand::Rules { command } => misc::run_rules_command(command),
        AppCommand::ScenarioCreate {
            name,
            force,
            watch_process,
            duration,
            preset,
            mangohud_log,
            notes,
        } => scenario::run_create_command(
            name,
            force,
            watch_process,
            duration,
            preset,
            mangohud_log,
            notes,
        ),
        AppCommand::ScenarioRun {
            name,
            role,
            dry_run,
            out_dir,
            mangohud_log_override,
        } => {
            scenario::run_scenario_command(name, role, dry_run, out_dir, mangohud_log_override)
                .await
        }
        AppCommand::ScenarioCompare {
            name,
            baseline,
            current,
            top,
            json_summary,
            validate,
        } => scenario::run_compare_command(name, baseline, current, top, json_summary, validate),
        AppCommand::ScenarioPath { name } => scenario::run_path_command(name),
        AppCommand::ScenarioList => scenario::run_list_command(),
    }
}
