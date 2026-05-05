mod affinity;
mod cli;
mod diagnosis;
mod ebpf_loader;
mod events;
mod hwmon;
mod mangohud;
mod metadata;
mod metrics;
mod process_tree;
mod profiles;
mod psi;
mod recorder;
mod report;
mod scorer;
mod scx;
mod session;
mod session_io;
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
        AppCommand::Restore { dry_run } => {
            let path = affinity::default_restore_path();
            if dry_run {
                print_restore_dry_run(&path)?;
            } else {
                let summary = affinity::restore_saved(&path)?;
                println!(
                    "restored {} affinity record(s); skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                    summary.restored,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch,
                    summary.legacy_unverified
                );
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
        AppCommand::Report {
            path,
            json,
            analysis_json,
            html,
            top,
            cluster_window_ms,
            diff,
            filter_class,
        } => {
            if let Some(diff_path) = diff {
                return report::print_diff_report(&path, &diff_path, top, filter_class);
            }
            if let Some(html_path) = html {
                report::write_html_report(&path, &html_path, top, cluster_window_ms, filter_class)?;
            }
            report::print_report(
                &path,
                json,
                analysis_json,
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
                mangohud_log,
                enforce,
                hwmon,
            })
            .await
        }
        AppCommand::Check {
            baseline,
            current,
            max_regression_p99_ms,
        } => report::check_percentile_regression(&baseline, &current, max_regression_p99_ms),
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
