use std::path::{Path, PathBuf};

use crate::{actions, affinity, audit, profile_restore};

pub fn run_restore_command(dry_run: bool) -> anyhow::Result<()> {
    let affinity_path = affinity::default_restore_path();
    let profile_path = profile_restore::default_restore_path();
    if dry_run {
        print_restore_dry_run(&affinity_path, &profile_path)?;
    } else {
        restore_profile_state(affinity_path, profile_path)?;
    }
    Ok(())
}

fn restore_profile_state(affinity_path: PathBuf, profile_path: PathBuf) -> anyhow::Result<()> {
    let mut summary = profile_restore::ProfileRestoreSummary::default();
    let mut restored_any = false;

    if affinity_path.exists() {
        match affinity::restore_saved(&affinity_path) {
            Ok(old_summary) => {
                restored_any = true;
                summary.affinity += old_summary.restored;
                summary.skipped_dead += old_summary.skipped_dead;
                summary.skipped_identity_mismatch += old_summary.skipped_identity_mismatch;
                summary.legacy_unverified += old_summary.legacy_unverified;
                summary.errors += old_summary.errors;
            }
            Err(err) => {
                audit::audit_or_warn(&audit::AuditEvent {
                    schema_version: 1,
                    unix_nanos: audit::unix_nanos_now(),
                    command: "restore".to_owned(),
                    action_id: Some("profile-restore".to_owned()),
                    safety_class: Some(actions::SafetyClass::ReversibleMediumRisk),
                    dry_run: false,
                    success: false,
                    affected_tasks: 0,
                    restore_path: Some(affinity_path.clone()),
                    action_phase: None,
                    error_category: None,
                    message: format!("restore failed: {err:#}"),
                });
                return Err(err);
            }
        }
    }

    if profile_path.exists() {
        match profile_restore::restore_saved(&profile_path) {
            Ok(profile_summary) => {
                restored_any = true;
                summary.affinity += profile_summary.affinity;
                summary.nice += profile_summary.nice;
                summary.ionice += profile_summary.ionice;
                summary.skipped_dead += profile_summary.skipped_dead;
                summary.skipped_identity_mismatch += profile_summary.skipped_identity_mismatch;
                summary.legacy_unverified += profile_summary.legacy_unverified;
                summary.errors += profile_summary.errors;
            }
            Err(err) => {
                audit::audit_or_warn(&audit::AuditEvent {
                    schema_version: 1,
                    unix_nanos: audit::unix_nanos_now(),
                    command: "restore".to_owned(),
                    action_id: Some("profile-restore".to_owned()),
                    safety_class: Some(actions::SafetyClass::ReversibleMediumRisk),
                    dry_run: false,
                    success: false,
                    affected_tasks: 0,
                    restore_path: Some(profile_path.clone()),
                    action_phase: None,
                    error_category: None,
                    message: format!("restore failed: {err:#}"),
                });
                return Err(err);
            }
        }
    }

    if restored_any {
        audit::audit_or_warn(&audit::AuditEvent {
            schema_version: 1,
            unix_nanos: audit::unix_nanos_now(),
            command: "restore".to_owned(),
            action_id: Some("profile-restore".to_owned()),
            safety_class: Some(actions::SafetyClass::ReversibleMediumRisk),
            dry_run: false,
            success: true,
            affected_tasks: summary.restored_total(),
            restore_path: Some(profile_path.clone()),
            action_phase: None,
            error_category: None,
            message: format!(
                "affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                summary.affinity,
                summary.nice,
                summary.ionice,
                summary.skipped_dead,
                summary.skipped_identity_mismatch,
                summary.legacy_unverified
            ),
        });
        println!(
            "restored profile state: affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={}",
            summary.affinity,
            summary.nice,
            summary.ionice,
            summary.skipped_dead,
            summary.skipped_identity_mismatch
        );
    } else {
        println!(
            "no restore file found at {} or {}",
            affinity_path.display(),
            profile_path.display()
        );
    }

    Ok(())
}

fn print_restore_dry_run(affinity_path: &Path, profile_path: &Path) -> anyhow::Result<()> {
    if !affinity_path.exists() && !profile_path.exists() {
        println!(
            "no restore file found at {} or {}",
            affinity_path.display(),
            profile_path.display()
        );
        return Ok(());
    }

    if affinity_path.exists() {
        let records = affinity::read_restore_records(affinity_path)?;
        println!(
            "found {} legacy affinity record(s) in {}",
            records.len(),
            affinity_path.display()
        );
        for record in records {
            println!(
                "tid={} process_pid={:?} mask={:?}",
                record.tid, record.process_pid, record.original_mask
            );
        }
    }

    if profile_path.exists() {
        let state = profile_restore::load_restore_state(profile_path)?;
        println!(
            "found profile restore state in {}: affinity={} nice={} ionice={}",
            profile_path.display(),
            state.affinity_records.len(),
            state.nice_records.len(),
            state.ionice_records.len()
        );
    }
    Ok(())
}
