use std::path::PathBuf;

use crate::{actions, affinity, audit, profile_restore};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileRestoreCommandOutcome {
    pub dry_run: bool,
    pub affinity_path: PathBuf,
    pub profile_path: PathBuf,
    pub affinity_records: usize,
    pub profile_affinity_records: usize,
    pub profile_nice_records: usize,
    pub profile_ionice_records: usize,
    pub restored_any: bool,
    pub summary: profile_restore::ProfileRestoreSummary,
    pub messages: Vec<String>,
}

impl ProfileRestoreCommandOutcome {
    pub fn found_any(&self) -> bool {
        self.restored_any
            || self.affinity_records > 0
            || self.profile_affinity_records > 0
            || self.profile_nice_records > 0
            || self.profile_ionice_records > 0
    }

    pub fn restored_total(&self) -> usize {
        self.summary.restored_total()
    }

    pub fn skipped_total(&self) -> usize {
        self.summary.skipped_dead + self.summary.skipped_identity_mismatch
    }

    pub fn unverified_total(&self) -> usize {
        self.summary.legacy_unverified
    }

    pub fn error_total(&self) -> usize {
        self.summary.errors
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreSummaryFields {
    pub status: String,
    pub restored_actions: usize,
    pub failed_actions: usize,
    pub skipped_actions: usize,
    pub profile_found: bool,
    pub profile_restored: usize,
    pub profile_skipped_total: usize,
    pub profile_skipped_dead: usize,
    pub profile_skipped_identity_mismatch: usize,
    pub profile_legacy_unverified: usize,
    pub profile_errors: usize,
}

impl RestoreSummaryFields {
    pub fn from_profile(
        status: impl Into<String>,
        restored_actions: usize,
        failed_actions: usize,
        skipped_actions: usize,
        profile: &ProfileRestoreCommandOutcome,
    ) -> Self {
        Self {
            status: status.into(),
            restored_actions,
            failed_actions,
            skipped_actions,
            profile_found: profile.found_any(),
            profile_restored: profile.restored_total(),
            profile_skipped_total: profile.skipped_total(),
            profile_skipped_dead: profile.summary.skipped_dead,
            profile_skipped_identity_mismatch: profile.summary.skipped_identity_mismatch,
            profile_legacy_unverified: profile.unverified_total(),
            profile_errors: profile.error_total(),
        }
    }

    pub fn render_fields(&self) -> String {
        format!(
            "status={} restored_actions={} failed_actions={} skipped_actions={} profile_found={} profile_restored={} profile_skipped_total={} profile_skipped_dead={} profile_skipped_identity_mismatch={} profile_legacy_unverified={} profile_errors={}",
            self.status,
            self.restored_actions,
            self.failed_actions,
            self.skipped_actions,
            self.profile_found,
            self.profile_restored,
            self.profile_skipped_total,
            self.profile_skipped_dead,
            self.profile_skipped_identity_mismatch,
            self.profile_legacy_unverified,
            self.profile_errors
        )
    }
}

pub fn run_restore_command(dry_run: bool) -> anyhow::Result<()> {
    let outcome = restore_profile_state_default(dry_run)?;
    for message in outcome.messages {
        println!("{message}");
    }
    Ok(())
}

pub fn restore_profile_state_default(
    dry_run: bool,
) -> anyhow::Result<ProfileRestoreCommandOutcome> {
    restore_profile_state_from_paths(
        affinity::default_restore_path(),
        profile_restore::default_restore_path(),
        dry_run,
    )
}

pub fn restore_profile_state_from_paths(
    affinity_path: PathBuf,
    profile_path: PathBuf,
    dry_run: bool,
) -> anyhow::Result<ProfileRestoreCommandOutcome> {
    if dry_run {
        return profile_restore_dry_run(affinity_path, profile_path);
    }

    let mut summary = profile_restore::ProfileRestoreSummary::default();
    let mut restored_any = false;
    let mut messages = Vec::new();

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
                "affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={} errors={}",
                summary.affinity,
                summary.nice,
                summary.ionice,
                summary.skipped_dead,
                summary.skipped_identity_mismatch,
                summary.legacy_unverified,
                summary.errors
            ),
        });
        messages.push(format!(
            "restored profile state: affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={} errors={}",
            summary.affinity,
            summary.nice,
            summary.ionice,
            summary.skipped_dead,
            summary.skipped_identity_mismatch,
            summary.legacy_unverified,
            summary.errors
        ));
    } else {
        messages.push(format!(
            "no restore file found at {} or {}",
            affinity_path.display(),
            profile_path.display()
        ));
    }

    Ok(ProfileRestoreCommandOutcome {
        dry_run,
        affinity_path,
        profile_path,
        restored_any,
        summary,
        messages,
        ..ProfileRestoreCommandOutcome::default()
    })
}

fn profile_restore_dry_run(
    affinity_path: PathBuf,
    profile_path: PathBuf,
) -> anyhow::Result<ProfileRestoreCommandOutcome> {
    let mut outcome = ProfileRestoreCommandOutcome {
        dry_run: true,
        affinity_path,
        profile_path,
        ..ProfileRestoreCommandOutcome::default()
    };

    collect_restore_dry_run(&mut outcome)?;
    Ok(outcome)
}

fn collect_restore_dry_run(outcome: &mut ProfileRestoreCommandOutcome) -> anyhow::Result<()> {
    let affinity_path = &outcome.affinity_path;
    let profile_path = &outcome.profile_path;

    if !affinity_path.exists() && !profile_path.exists() {
        outcome.messages.push(format!(
            "no restore file found at {} or {}",
            affinity_path.display(),
            profile_path.display()
        ));
        return Ok(());
    }

    if affinity_path.exists() {
        let records = affinity::read_restore_records(affinity_path)?;
        outcome.affinity_records = records.len();
        outcome.messages.push(format!(
            "found {} legacy affinity record(s) in {}",
            records.len(),
            affinity_path.display()
        ));
        for record in records {
            outcome.messages.push(format!(
                "tid={} process_pid={:?} mask={:?}",
                record.tid, record.process_pid, record.original_mask
            ));
        }
    }

    if profile_path.exists() {
        let state = profile_restore::load_restore_state(profile_path)?;
        outcome.profile_affinity_records = state.affinity_records.len();
        outcome.profile_nice_records = state.nice_records.len();
        outcome.profile_ionice_records = state.ionice_records.len();
        outcome.messages.push(format!(
            "found profile restore state in {}: affinity={} nice={} ionice={}",
            profile_path.display(),
            state.affinity_records.len(),
            state.nice_records.len(),
            state.ionice_records.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-restore-command-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn profile_restore_dry_run_reports_all_known_restore_files() {
        let dir = temp_dir("dry-run");
        let affinity_path = dir.join("last_affinity_restore.json");
        let profile_path = dir.join("last_profile_restore.json");

        affinity::save_restore_state(
            &affinity_path,
            &[affinity::AffinityRecord {
                tid: 123.into(),
                process_pid: Some(123.into()),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                original_mask: affinity::CpuMask::parse("0").unwrap(),
                applied_mask: affinity::CpuMask::parse("1").unwrap(),
            }],
        )
        .unwrap();
        profile_restore::save_restore_state(
            &profile_path,
            &profile_restore::ProfileRestoreState {
                schema_version: profile_restore::PROFILE_RESTORE_SCHEMA_VERSION,
                affinity_records: Vec::new(),
                nice_records: vec![profile_restore::NiceRestoreRecordV2 {
                    tid: 123.into(),
                    process_pid: Some(123.into()),
                    process_starttime_ticks: None,
                    task_starttime_ticks: None,
                    comm: Some("game".to_owned()),
                    original_nice: 0,
                    applied_nice: -5,
                }],
                ionice_records: Vec::new(),
            },
        )
        .unwrap();

        let outcome =
            restore_profile_state_from_paths(affinity_path.clone(), profile_path.clone(), true)
                .unwrap();

        assert!(outcome.found_any());
        assert_eq!(outcome.affinity_records, 1);
        assert_eq!(outcome.profile_nice_records, 1);
        assert!(outcome.messages.iter().any(|message| {
            message.contains("found 1 legacy affinity record(s)")
                && message.contains(&affinity_path.display().to_string())
        }));
        assert!(outcome.messages.iter().any(|message| {
            message.contains("found profile restore state")
                && message.contains("affinity=0 nice=1 ionice=0")
        }));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn profile_restore_dry_run_reports_missing_restore_files() {
        let dir = temp_dir("missing");
        let affinity_path = dir.join("missing-affinity.json");
        let profile_path = dir.join("missing-profile.json");

        let outcome =
            restore_profile_state_from_paths(affinity_path.clone(), profile_path.clone(), true)
                .unwrap();

        assert!(!outcome.found_any());
        assert_eq!(outcome.messages.len(), 1);
        assert!(outcome.messages[0].contains(&affinity_path.display().to_string()));
        assert!(outcome.messages[0].contains(&profile_path.display().to_string()));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn profile_restore_outcome_found_any_treats_successful_restore_as_found() {
        let outcome = ProfileRestoreCommandOutcome {
            restored_any: true,
            ..ProfileRestoreCommandOutcome::default()
        };

        assert!(outcome.found_any());
    }

    #[test]
    fn restore_summary_fields_render_profile_restore_counts() {
        let outcome = ProfileRestoreCommandOutcome {
            restored_any: true,
            summary: profile_restore::ProfileRestoreSummary {
                affinity: 1,
                nice: 2,
                ionice: 3,
                skipped_dead: 4,
                skipped_identity_mismatch: 5,
                legacy_unverified: 6,
                errors: 7,
            },
            ..ProfileRestoreCommandOutcome::default()
        };

        assert_eq!(outcome.restored_total(), 6);
        assert_eq!(outcome.skipped_total(), 9);
        assert_eq!(outcome.unverified_total(), 6);
        assert_eq!(outcome.error_total(), 7);

        let summary = RestoreSummaryFields::from_profile("Restored", 8, 9, 10, &outcome);
        let rendered = summary.render_fields();

        assert!(rendered.contains("status=Restored"));
        assert!(rendered.contains("restored_actions=8"));
        assert!(rendered.contains("failed_actions=9"));
        assert!(rendered.contains("skipped_actions=10"));
        assert!(rendered.contains("profile_found=true"));
        assert!(rendered.contains("profile_restored=6"));
        assert!(rendered.contains("profile_skipped_total=9"));
        assert!(rendered.contains("profile_skipped_dead=4"));
        assert!(rendered.contains("profile_skipped_identity_mismatch=5"));
        assert!(rendered.contains("profile_legacy_unverified=6"));
        assert!(rendered.contains("profile_errors=7"));
    }
}
