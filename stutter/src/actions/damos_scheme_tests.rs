use std::fs;

use super::*;

fn admin(name: &str, state: &str, contexts: u32) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-damos-{name}-{}", std::process::id()));
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
    fs::write(kd.join("0/state"), state).unwrap();
    fs::write(kd.join("0/contexts/nr_contexts"), contexts.to_string()).unwrap();
    root
}

fn request(action: DamosAction) -> DamosRequest {
    DamosRequest {
        task: 4242,
        start_ticks: 777,
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

fn alive(task: u32) -> Option<u64> {
    (task == 4242).then_some(777)
}

fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join("kdamonds").join(file))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn a_monitoring_scheme_commits_reads_back_and_clears() {
    let root = admin("stat", "off", 0);
    let prepared = prepare(&root, &request(DamosAction::Stat), alive).unwrap();
    assert_eq!(read(&root, "0/state"), "off", "prepare writes nothing");
    let committed = prepared.commit().unwrap();
    assert_eq!(read(&root, "0/state"), "on");
    assert_eq!(read(&root, "0/contexts/0/targets/0/pid_target"), "4242");
    assert_eq!(
        committed.restore(),
        DamosRestore::Cleared {
            irreversible_effect_possible: false
        }
    );
    assert_eq!(read(&root, "0/state"), "off");
    assert_eq!(read(&root, "0/contexts/nr_contexts"), "0");
}

#[test]
fn reclaim_is_classed_irreversible_and_needs_acknowledgement_and_a_bounded_quota() {
    let root = admin("pageout", "off", 0);
    let mut unacknowledged = request(DamosAction::Pageout);
    unacknowledged.acknowledge_irreversible = false;
    assert_eq!(
        prepare(&root, &unacknowledged, alive).unwrap_err(),
        DamosDenial::IrreversibleNotAcknowledged
    );
    let mut unbounded = request(DamosAction::Pageout);
    unbounded.quota.ms = 0;
    assert_eq!(
        prepare(&root, &unbounded, alive).unwrap_err(),
        DamosDenial::QuotaUnbounded
    );
    unbounded.quota.ms = 500;
    assert_eq!(
        prepare(&root, &unbounded, alive).unwrap_err(),
        DamosDenial::QuotaUnbounded
    );
    let committed = prepare(&root, &request(DamosAction::Pageout), alive)
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(
        committed.restore(),
        DamosRestore::Cleared {
            irreversible_effect_possible: true
        },
        "configuration restores; the reclaim already done does not"
    );
}

#[test]
fn another_owners_kdamond_and_a_reused_pid_are_never_used() {
    let busy = admin("busy", "on", 1);
    assert_eq!(
        prepare(&busy, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::NoIdleKdamond
    );
    let configured = admin("configured", "off", 2);
    assert_eq!(
        prepare(&configured, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::NoIdleKdamond
    );
    let idle = admin("reuse", "off", 0);
    assert_eq!(
        prepare(&idle, &request(DamosAction::Stat), |_| Some(1)).unwrap_err(),
        DamosDenial::TargetIdentity
    );
    let absent = std::env::temp_dir().join("stutter-damos-absent");
    assert_eq!(
        prepare(&absent, &request(DamosAction::Stat), alive).unwrap_err(),
        DamosDenial::Unavailable
    );
    let mut region = request(DamosAction::Stat);
    region.min_region_bytes = 0;
    assert_eq!(
        prepare(&idle, &region, alive).unwrap_err(),
        DamosDenial::BadRegion
    );
}

#[test]
fn restore_leaves_a_kdamond_someone_else_reconfigured() {
    let root = admin("takeover", "off", 0);
    let committed = prepare(&root, &request(DamosAction::Stat), alive)
        .unwrap()
        .commit()
        .unwrap();
    fs::write(
        root.join("kdamonds/0/contexts/0/targets/0/pid_target"),
        "999",
    )
    .unwrap();
    assert_eq!(committed.restore(), DamosRestore::ExternallyChanged);
    assert_eq!(
        read(&root, "0/state"),
        "on",
        "not stopped: it is no longer ours"
    );
}
