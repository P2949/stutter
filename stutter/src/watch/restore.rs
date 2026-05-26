pub fn restore_profile_watch_on_exit() -> anyhow::Result<()> {
    let path = crate::profile_restore::default_restore_path();
    if !path.exists() {
        println!("stopped profile watch; no restore file was written");
        return Ok(());
    }

    match crate::profile_restore::restore_saved(&path) {
        Ok(summary) => {
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "apply-profile --watch restore".to_owned(),
                action_id: Some("profile-restore".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleMediumRisk),
                dry_run: false,
                success: true,
                affected_tasks: summary.restored_total(),
                restore_path: Some(path.clone()),
                action_phase: None,
                error_category: None,
                message: format!(
                    "watch restore completed affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={}",
                    summary.affinity,
                    summary.nice,
                    summary.ionice,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch
                ),
            });
            println!(
                "stopped profile watch; restored profile state: affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={}",
                summary.affinity,
                summary.nice,
                summary.ionice,
                summary.skipped_dead,
                summary.skipped_identity_mismatch
            );
        }
        Err(err) => {
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "apply-profile --watch restore".to_owned(),
                action_id: Some("profile-restore".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleMediumRisk),
                dry_run: false,
                success: false,
                affected_tasks: 0,
                restore_path: Some(path.clone()),
                action_phase: None,
                error_category: None,
                message: format!("restore failed: {err:#}"),
            });
            return Err(err);
        }
    }

    Ok(())
}
