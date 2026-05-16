use super::helpers::daemon_state_store_for_path;
use crate::{
    affinity,
    autotune::emergency_restore::{
        AutotuneRestoreCommandInput, AutotuneRestoreOutcome, AutotuneRestoreStatus,
        restore_known_autotune_actions,
    },
    commands::restore,
    daemon::default_daemon_state_snapshot_path,
    profile_restore,
};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DaemonRestoreCommandOutcome {
    pub autotune: AutotuneRestoreOutcome,
    pub profile: restore::ProfileRestoreCommandOutcome,
}

pub fn run_restore_command(
    input: crate::commands::input::DaemonRestoreCommandInput,
) -> anyhow::Result<()> {
    run_restore_command_with_profile_paths(
        input,
        None,
        None,
        None,
        affinity::default_restore_path(),
        profile_restore::default_restore_path(),
    )?;
    Ok(())
}

pub fn run_restore_command_with_profile_paths(
    input: crate::commands::input::DaemonRestoreCommandInput,
    journal_path: Option<std::path::PathBuf>,
    audit_path: Option<std::path::PathBuf>,
    history_path: Option<std::path::PathBuf>,
    affinity_path: std::path::PathBuf,
    profile_path: std::path::PathBuf,
) -> anyhow::Result<DaemonRestoreCommandOutcome> {
    let outcome = restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path,
        audit_path,
        history_path,
        dry_run: input.dry_run,
    })?;

    for message in &outcome.messages {
        println!("{message}");
    }

    let profile_outcome =
        restore::restore_profile_state_from_paths(affinity_path, profile_path, input.dry_run)?;
    for message in &profile_outcome.messages {
        println!("{message}");
    }
    let restore_summary = daemon_restore_summary_fields(&outcome, &profile_outcome);
    println!("daemon restore summary: {restore_summary}");

    if input.dry_run {
        return Ok(DaemonRestoreCommandOutcome {
            autotune: outcome,
            profile: profile_outcome,
        });
    }

    let state_path = default_daemon_state_snapshot_path();
    let mut store = daemon_state_store_for_path(&state_path)?;
    let command = if input.emergency {
        "daemon_emergency_restore"
    } else {
        "daemon_restore"
    };

    match outcome.status {
        AutotuneRestoreStatus::Clean | AutotuneRestoreStatus::Restored => {
            store.mark_restored(format!("{command} completed {restore_summary}"))?;
            println!(
                "daemon restore state updated; state_path={}",
                state_path.display()
            );
            Ok(DaemonRestoreCommandOutcome {
                autotune: outcome,
                profile: profile_outcome,
            })
        }
        AutotuneRestoreStatus::ApplyingWithoutRollbackToken | AutotuneRestoreStatus::Faulted => {
            store.mark_fault(
                store.state().mode,
                format!("{command} could not complete {restore_summary}"),
                Some("stutter daemon emergency-restore --dry-run".to_owned()),
            )?;
            anyhow::bail!(
                "daemon restore did not complete safely; status={:?}",
                outcome.status
            );
        }
        AutotuneRestoreStatus::DryRun => Ok(DaemonRestoreCommandOutcome {
            autotune: outcome,
            profile: profile_outcome,
        }),
    }
}

pub fn daemon_restore_summary_fields(
    outcome: &AutotuneRestoreOutcome,
    profile_outcome: &restore::ProfileRestoreCommandOutcome,
) -> String {
    restore::RestoreSummaryFields::from_profile(
        format!("{:?}", outcome.status),
        outcome.restored_actions,
        outcome.failed_actions,
        outcome.skipped_actions,
        profile_outcome,
    )
    .render_fields()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_restore_dry_run_discovers_autotune_and_profile_restore_records() {
        let output = run_restore_command_with_profile_paths(
            crate::commands::input::DaemonRestoreCommandInput {
                dry_run: true,
                emergency: false,
            },
            None,
            None,
            None,
            "/dev/null/affinity".into(),
            "/dev/null/profile".into(),
        )
        .unwrap();

        assert_eq!(output.autotune.status, AutotuneRestoreStatus::Clean);
        assert!(!output.profile.restored_any);
    }

    #[test]
    fn daemon_restore_summary_includes_autotune_and_profile_counts() {
        let outcome = AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::Restored,
            restored_actions: 1,
            failed_actions: 2,
            skipped_actions: 3,
            messages: vec!["autotune-message".to_owned()],
        };
        let profile_outcome = restore::ProfileRestoreCommandOutcome {
            restored_any: true,
            summary: crate::profile_restore::ProfileRestoreSummary {
                affinity: 1,
                nice: 2,
                ionice: 1,
                ..Default::default()
            },
            ..Default::default()
        };

        let summary = daemon_restore_summary_fields(&outcome, &profile_outcome);

        assert!(summary.contains("status=Restored"));
        assert!(summary.contains("restored_actions=1"));
        assert!(summary.contains("failed_actions=2"));
        assert!(summary.contains("skipped_actions=3"));
        assert!(summary.contains("profile_restored=4"));
        assert!(summary.contains("profile_errors=0"));
        assert!(summary.contains("profile_skipped_total=0"));
    }
}
