mod actions;
mod advisor;
mod affinity;
mod audit;
mod cli;
mod diagnosis;
mod doctor;
mod ebpf_loader;
mod events;
mod hwmon;
mod mangohud;
mod metadata;
mod metrics;
mod perf_counters;
mod process_tree;
mod profiles;
mod psi;
mod recommend;
mod recorder;
mod report;
mod scorer;
mod scx;
mod session;
mod session_io;
mod summary;
mod tasks;
mod tui;
mod tune;
mod watch;

#[cfg(test)]
mod recording_fixture_tests;
#[cfg(test)]
mod regression_tests;

use std::path::Path;

use cli::{AppCommand, parse_app_command};
use session::run_monitor;
use tune::{TuneCommandInput, tune_command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => run_monitor(config, None).await,
        AppCommand::Bench {
            config,
            role,
            run_name,
        } => {
            run_monitor(config, None).await?;
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
        AppCommand::Audit { path, tail, json } => {
            audit::audit_command(audit::AuditCommandInput { path, tail, json })
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
        AppCommand::ProfileTemplate { topology } => {
            if topology {
                print!("{}", profiles::generate_topology_template());
                Ok(())
            } else {
                anyhow::bail!("profile-template requires --topology");
            }
        }
    }
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
