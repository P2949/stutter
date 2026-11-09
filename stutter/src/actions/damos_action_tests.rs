use std::fs;

use super::*;
use crate::actions::damos_scheme::{DamosAction, DamosQuota};

fn admin(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("stutter-damos-act-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let kd = root.join("kdamonds");
    let ctx = kd.join("0/contexts/0");
    for dir in [
        ctx.join("targets/0"),
        ctx.join("schemes/0/quotas"),
        ctx.join("schemes/0/access_pattern/sz"),
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(kd.join("nr_kdamonds"), "1").unwrap();
    fs::write(kd.join("0/state"), "off").unwrap();
    fs::write(kd.join("0/contexts/nr_contexts"), "0").unwrap();
    root
}

/// A request naming this very test process, with its real start time.
fn request(action: DamosAction) -> DamosRequest {
    let task = std::process::id();
    let live =
        crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), task).unwrap();
    DamosRequest {
        task,
        start_ticks: live.start_ticks,
        action,
        quota: DamosQuota {
            ms: 5,
            bytes: 1 << 20,
            reset_interval_ms: 1000,
        },
        acknowledge_irreversible: true,
        min_region_bytes: 4096,
        max_region_bytes: 1 << 30,
    }
}

fn make(name: &str, damos: DamosAction) -> DamosSchemeAction {
    DamosSchemeAction {
        admin: admin(name),
        request: request(damos),
    }
}

#[test]
fn the_default_policy_denies_and_the_action_is_high_risk_system_wide() {
    let action = make("deny", DamosAction::Stat);
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(action.apply().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    assert!(action.descriptor().touches_system_wide_state);
    assert!(!action.descriptor().persistent_effect);
    assert!(
        make("deny-reclaim", DamosAction::Pageout)
            .descriptor()
            .persistent_effect,
        "reclaim is an irreversible persistent effect"
    );
}

#[test]
fn an_allowed_monitoring_scheme_runs_and_rollback_tears_it_down() {
    let action = make("cycle", DamosAction::Stat);
    let policy = DamosActionPolicy { allow: true };
    let token = action.apply_with_policy(&policy).unwrap();
    assert_eq!(token.kind(), "damos-teardown");
    assert!(action.verify().unwrap().applied);
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    action.rollback(&token).unwrap();
    assert!(!action.verify().unwrap().applied);
    // Rolling back again finds an idle kdamond: nothing to do.
    action.rollback(&token).unwrap();
}

#[test]
fn a_reused_pid_and_an_unacknowledged_reclaim_are_refused() {
    let mut reused = make("reuse", DamosAction::Stat);
    reused.request.start_ticks += 1;
    assert!(
        reused
            .apply_with_policy(&DamosActionPolicy { allow: true })
            .is_err()
    );
    let mut unacknowledged = make("noack", DamosAction::Pageout);
    unacknowledged.request.acknowledge_irreversible = false;
    assert!(
        unacknowledged
            .apply_with_policy(&DamosActionPolicy { allow: true })
            .is_err()
    );
}

#[test]
fn rollback_leaves_a_kdamond_someone_else_reconfigured_and_refuses_a_foreign_token() {
    let action = make("takeover", DamosAction::Stat);
    let token = action
        .apply_with_policy(&DamosActionPolicy { allow: true })
        .unwrap();
    fs::write(
        action
            .admin
            .join("kdamonds/0/contexts/0/targets/0/pid_target"),
        "1",
    )
    .unwrap();
    assert!(action.rollback(&token).is_err());
    assert!(
        action.verify().unwrap().applied,
        "not stopped: no longer ours"
    );
    assert!(
        action
            .rollback(&RollbackToken::NiceRestore { records: vec![] })
            .is_err()
    );
}

#[test]
fn recovery_is_manual_only_and_says_pages_are_not_restored() {
    let action = make("recovery", DamosAction::Pageout);
    let token = action
        .apply_with_policy(&DamosActionPolicy { allow: true })
        .unwrap();
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::ManualOnly
    );
    assert!(descriptor.identity_contract.contains("irreversible"));
    let summary = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap();
    assert_eq!(summary.restored_items, 1);
    assert!(summary.messages.iter().any(|m| m.contains("not restored")));
}

/// Live: the real DAMON admin sysfs, run as root. One kdamond is created for the test, a
/// monitoring-only (`stat`) scheme is configured for this very process, read back as running,
/// torn down through rollback, and the kdamond is removed again. Nothing is reclaimed or moved.
#[test]
#[ignore = "live: needs root and /sys/kernel/mm/damon/admin"]
fn live_real_damon_stat_scheme_runs_and_tears_down() {
    let admin = PathBuf::from(DAMON_ADMIN_ROOT);
    let count = admin.join("kdamonds/nr_kdamonds");
    assert_eq!(
        fs::read_to_string(&count)
            .expect("BlockedResource: DAMON sysfs")
            .trim(),
        "0",
        "another owner has kdamonds: not touched"
    );
    fs::write(&count, "1").unwrap();
    let action = DamosSchemeAction {
        admin: admin.clone(),
        request: request(DamosAction::Stat),
    };
    let outcome = (|| {
        let token = action.apply_with_policy(&DamosActionPolicy { allow: true })?;
        let running = fs::read_to_string(admin.join("kdamonds/0/state"))?;
        action.rollback(&token)?;
        let stopped = fs::read_to_string(admin.join("kdamonds/0/state"))?;
        anyhow::Ok((running.trim().to_owned(), stopped.trim().to_owned()))
    })();
    // Remove the kdamond whatever happened, so nothing is left running.
    let _ = fs::write(admin.join("kdamonds/0/state"), "off");
    fs::write(&count, "0").unwrap();
    {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "damon live: {outcome:?}");
    }
    match outcome {
        Ok((running, stopped)) => {
            assert_eq!(running, "on");
            assert_eq!(stopped, "off");
        }
        // The kernel may refuse (for example because an in-kernel DAMON module already owns a
        // kdamond): that is a typed, cleanly rolled-back refusal, never a half-configured kdamond.
        Err(error) => {
            assert!(error.to_string().contains("not in effect"), "{error}");
            assert_eq!(fs::read_to_string(&count).unwrap().trim(), "0");
        }
    }
}
