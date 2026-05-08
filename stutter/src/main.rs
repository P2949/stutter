mod actions;
mod advisor;
mod affinity;
mod agent;
#[cfg(test)]
mod artifact_contract_tests;
mod audit;
mod autotune;
mod cli;
mod config_file;

mod diagnosis;
mod doctor;
mod ebpf_loader;
mod events;
mod flamegraph;
mod focus;
mod hwmon;
mod irq_inspect;
mod mangohud;
mod metadata;
mod metrics;
mod otel;
mod perf_counters;
mod presets;
mod probe_catalog;
mod process_tree;
mod profiles;
mod prometheus;
mod psi;
mod recommend;
mod recorder;
mod remote;
mod report;
mod scenario;
mod scorer;
mod scx;
mod session;
mod session_events;
mod session_io;
mod summary;
mod tasks;
mod topology;
mod tui;
mod tune;
mod validate;
mod watch;

#[cfg(test)]
mod recording_fixture_tests;
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod runnable_depth_tests;
#[cfg(test)]
mod test_fixture_builder;
#[cfg(test)]
mod validation_corpus_tests;

use std::{path::Path, sync::Arc};

use cli::{AppCommand, parse_app_command};
use session::run_monitor;
use tune::{TuneCommandInput, tune_command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => {
            if let Some(remote) = config.remote.as_deref() {
                let request = remote::request_from_monitor_config(&config)?;
                remote::run_remote_monitor(remote, request).await?;
                Ok(())
            } else {
                run_monitor(config, None, None, None).await.map(|_| ())
            }
        }
        AppCommand::Bench {
            config,
            role,
            run_name,
        } => {
            run_monitor(config, None, None, None).await?;
            if role == "baseline" {
                println!(
                    "bench complete role=baseline run_name={} next=\"run tune, then stutter recommend --baseline <run-dir> --tune <tune-dir>\"",
                    run_name
                );
            } else {
                println!(
                    "bench complete role=current run_name={} next=\"use stutter report --diff <baseline-run-dir> <current-run-dir>\"",
                    run_name
                );
            }
            Ok(())
        }
        AppCommand::Restore { dry_run } => {
            let path = affinity::default_restore_path();
            if dry_run {
                print_restore_dry_run(&path)?;
            } else {
                match affinity::restore_saved(&path) {
                    Ok(summary) => {
                        audit::audit_or_warn(&audit::AuditEvent {
                            schema_version: 1,
                            unix_nanos: audit::unix_nanos_now(),
                            command: "restore".to_owned(),
                            action_id: Some("cpu-affinity-restore".to_owned()),
                            safety_class: Some(actions::SafetyClass::ReversibleLowRisk),
                            dry_run: false,
                            success: true,
                            affected_tasks: summary.restored,
                            restore_path: Some(path.clone()),
                            message: format!(
                                "restored={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                                summary.restored,
                                summary.skipped_dead,
                                summary.skipped_identity_mismatch,
                                summary.legacy_unverified
                            ),
                        });
                        println!(
                            "restored {} affinity record(s); skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                            summary.restored,
                            summary.skipped_dead,
                            summary.skipped_identity_mismatch,
                            summary.legacy_unverified
                        );
                    }
                    Err(err) => {
                        audit::audit_or_warn(&audit::AuditEvent {
                            schema_version: 1,
                            unix_nanos: audit::unix_nanos_now(),
                            command: "restore".to_owned(),
                            action_id: Some("cpu-affinity-restore".to_owned()),
                            safety_class: Some(actions::SafetyClass::ReversibleLowRisk),
                            dry_run: false,
                            success: false,
                            affected_tasks: 0,
                            restore_path: Some(path.clone()),
                            message: format!("restore failed: {err:#}"),
                        });
                        return Err(err);
                    }
                }
            }
            Ok(())
        }
        AppCommand::ApplyProfile {
            tree_pid,
            profile,
            force,
            dry_run,
            watch,
            keep_applied,
            refresh_ms,
            enforce,
        } => {
            watch::apply_profile_command(watch::ApplyProfileCommandInput {
                tree_pid,
                profile_path: profile,
                force,
                dry_run,
                watch,
                keep_applied,
                refresh_ms,
                enforce,
            })
            .await
        }
        AppCommand::InspectTree { tree_pid } => {
            let rendered = process_tree::render_tree(tree_pid)?;
            print!("{rendered}");
            Ok(())
        }
        AppCommand::Summary {
            path,
            json,
            top,
            filter_class,
        } => summary::summary_command(&path, json, top, filter_class),
        AppCommand::Validate { path, json, strict } => {
            validate::validate_command(validate::ValidateCommandInput { path, json, strict })
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
            flamegraph: flamegraph_path,
        } => {
            if let Some(batch_dir) = batch {
                return report::print_batch_report(
                    &batch_dir,
                    diff.as_deref(),
                    json_summary || json,
                    top,
                    filter_class,
                );
            }
            let Some(path) = path else {
                anyhow::bail!("report requires PATH unless --batch is set");
            };
            if let Some(diff_path) = diff {
                return report::print_diff_report(&diff_path, &path, top, filter_class);
            }
            if let Some(html_path) = html {
                report::write_html_report(&path, &html_path, top, cluster_window_ms, filter_class)?;
            }
            report::print_report(
                &path,
                json,
                analysis_json,
                json_summary,
                top,
                cluster_window_ms,
                filter_class,
                flamegraph_path,
            )
        }
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
            tune_command(TuneCommandInput {
                tree_pid,
                profiles_path: profiles,
                epoch_seconds,
                warmup_seconds,
                runs,
                keep_best,
                baseline_profile,
                out_dir,
                mangohud_log,
                enforce,
                hwmon,
            })
            .await
        }
        AppCommand::Recommend {
            baseline,
            tune,
            json,
            markdown,
        } => recommend::recommend_command(recommend::RecommendCommandInput {
            baseline,
            tune,
            json,
            markdown,
        }),
        AppCommand::Check {
            baseline,
            current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        } => report::check_regression(
            &baseline,
            &current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        ),
        AppCommand::Autotune { input } => autotune::autotune_command(input).await,
        AppCommand::AutotuneStatus { json } => autotune::status::autotune_status_command(
            autotune::status::AutotuneStatusCommandInput {
                json,
                history_path: None,
            },
        ),
        AppCommand::AutotuneReplayHistory { history } => {
            autotune::history_replay::autotune_replay_history_command(
                autotune::history_replay::AutotuneReplayHistoryCommandInput {
                    history_path: history,
                },
            )
        }
        AppCommand::Audit { path, tail, json } => {
            audit::audit_command(audit::AuditCommandInput { path, tail, json })
        }
        AppCommand::AutotuneReplay { run, config } => {
            autotune::replay::replay_command(autotune::replay::AutotuneReplayInput {
                run_dir: run,
                config_path: config,
            })
        }
        AppCommand::Advisor {
            run,
            profiles,
            json,
            watch_runs,
            runs_dir,
            poll_seconds,
            once,
        } => {
            advisor::advisor_command(advisor::AdvisorCommandInput {
                run,
                profiles,
                json,
                watch_runs,
                runs_dir,
                poll_seconds,
                once,
            })
            .await
        }
        AppCommand::Doctor { input } => doctor::doctor_command(input),
        AppCommand::Probes { json } => probe_catalog::probes_command(json),
        AppCommand::ProfileTemplate { topology } => {
            if topology {
                print!("{}", profiles::generate_topology_template());
                Ok(())
            } else {
                anyhow::bail!("profile-template requires --topology");
            }
        }
        AppCommand::InspectIrqs { json, filter, top } => {
            irq_inspect::run_inspect_irqs(json, &filter, top)
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
            let runs_dir = runs_dir.unwrap_or_else(agent::default_runs_dir);
            let bearer_token =
                agent::load_bearer_token(&bearer_token_env, bearer_token_file.as_deref())?;

            let user_config = config_file::load_user_config()?;
            let autotune_limits =
                config_file::agent_autotune_limits_from_user_config(user_config.as_ref())?;

            agent::run_agent(agent::AgentConfig {
                bind,
                runs_dir,
                allow_unsafe_bind,
                bearer_token,
                max_duration_seconds,
                max_targets,
                max_concurrent_recordings,
                autotune_limits,
            })
            .await
        }
        AppCommand::Completions { shell } => {
            let mut cmd = cli::command();
            clap_complete::generate(shell, &mut cmd, "stutter", &mut std::io::stdout());
            Ok(())
        }
        AppCommand::Man { output } => {
            render_man_page(output.as_deref())?;
            Ok(())
        }
        AppCommand::ScenarioCreate {
            name,
            force,
            watch_process,
            duration,
            preset,
            mangohud_log,
            notes,
        } => {
            let path = scenario::create_scenario(scenario::ScenarioCreateInput {
                name: name.clone(),
                force,
                watch_process,
                duration,
                preset,
                mangohud_log,
                notes,
            })?;
            println!("created scenario {} at {}", name, path.display());
            println!("edit notes/expected_classes before running if needed");
            Ok(())
        }
        AppCommand::ScenarioRun {
            name,
            role,
            dry_run,
            out_dir,
            mangohud_log_override,
        } => {
            let role = scenario::ScenarioRole::parse(&role)?;
            let prepared = scenario::prepare_scenario_run(scenario::ScenarioRunInput {
                name,
                role,
                dry_run,
                out_dir,
                mangohud_log_override,
            })?;

            if prepared.dry_run {
                print!("{}", prepared.dry_run_text);
                return Ok(());
            }

            println!("{}", prepared.start_text);
            let record = prepared.record.clone();
            let config = Arc::new(prepared.config);

            match run_monitor(config, None, None, None).await {
                Ok(_) => {
                    scenario::append_run_record(&record)?;
                    println!("scenario run complete: {}", record.run_dir.display());
                    Ok(())
                }
                Err(err) => {
                    // Optional: audit_scenario_run_failure
                    Err(err)
                }
            }
        }
        AppCommand::ScenarioCompare {
            name,
            baseline,
            current,
            top,
            json_summary,
            validate,
        } => scenario::compare_scenario(scenario::ScenarioCompareInput {
            name,
            baseline,
            current,
            top,
            json_summary,
            validate,
        }),
        AppCommand::ScenarioPath { name } => {
            let path = scenario::scenario_path(&name)?;
            println!("{}", path.display());
            Ok(())
        }
        AppCommand::ScenarioList => scenario::list_scenarios(),
    }
}

fn render_man_page(output: Option<&std::path::Path>) -> anyhow::Result<()> {
    use anyhow::Context;

    let cmd = cli::command();
    let man = clap_mangen::Man::new(cmd);

    if let Some(path) = output {
        let mut file = std::fs::File::create(path)
            .with_context(|| format!("failed to create man page {}", path.display()))?;
        man.render(&mut file)
            .with_context(|| format!("failed to render man page to {}", path.display()))?;
    } else {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        man.render(&mut handle)
            .with_context(|| "failed to render man page to stdout")?;
    }

    Ok(())
}

fn print_restore_dry_run(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        println!("no restore file found at {}", path.display());
        return Ok(());
    }

    let records = affinity::read_restore_records(path)?;
    println!(
        "found {} affinity record(s) in {}",
        records.len(),
        path.display()
    );
    for record in records {
        println!(
            "tid={} process_pid={:?} mask={:?}",
            record.tid, record.process_pid, record.original_mask
        );
    }
    Ok(())
}
