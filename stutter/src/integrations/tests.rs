use std::fs;

use super::*;

fn root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-owners-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn process(proc_root: &std::path::Path, pid: u32, comm: &str) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
}

#[test]
fn a_quiet_host_has_no_external_owner() {
    let base = root("quiet");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 10, "bash");
    fs::create_dir_all(sys.join("kernel/sched_ext")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), "disabled\n").unwrap();
    let report = probe(&proc_root, &sys);
    for domain in [
        ResourceDomain::Scheduler,
        ResourceDomain::GpuPower,
        ResourceDomain::TaskPriorityAndPower,
    ] {
        assert_eq!(report.external_owner(domain), None, "{domain:?}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn each_owner_claims_only_its_own_domain() {
    let base = root("owners");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 11, "gamemoded");
    process(&proc_root, 12, "lactd");
    fs::create_dir_all(sys.join("kernel/sched_ext/root")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), "enabled\n").unwrap();
    fs::write(sys.join("kernel/sched_ext/root/ops"), "scx_lavd\n").unwrap();
    let report = probe(&proc_root, &sys);
    assert_eq!(
        report.sched_ext,
        SchedExt::Enabled {
            ops: "scx_lavd".into()
        }
    );
    assert!(report.external_owner(ResourceDomain::Scheduler).is_some());
    assert!(
        report
            .external_owner(ResourceDomain::GpuPower)
            .unwrap()
            .contains("LACT")
    );
    assert!(
        report
            .external_owner(ResourceDomain::TaskPriorityAndPower)
            .unwrap()
            .contains("GameMode")
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_manager_without_a_loaded_scheduler_still_owns_the_scheduler() {
    let base = root("manager");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 13, "scx_loader");
    fs::create_dir_all(sys.join("kernel/sched_ext")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), "disabled\n").unwrap();
    assert!(
        probe(&proc_root, &sys)
            .external_owner(ResourceDomain::Scheduler)
            .is_some()
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn unreadable_state_fails_closed() {
    let base = root("unknown");
    let sys = base.join("sys");
    fs::create_dir_all(sys.join("kernel/sched_ext")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), "garbage\n").unwrap();
    let report = probe(&base.join("missing-proc"), &sys);
    assert_eq!(report.sched_ext, SchedExt::Unknown);
    assert_eq!(report.gamemode, Daemon::Unknown);
    for domain in [
        ResourceDomain::Scheduler,
        ResourceDomain::GpuPower,
        ResourceDomain::TaskPriorityAndPower,
    ] {
        assert!(report.external_owner(domain).is_some(), "{domain:?}");
    }
    // A kernel without sched_ext is not an owner.
    let none = probe(&base.join("missing-proc"), &base.join("no-sys"));
    assert_eq!(none.sched_ext, SchedExt::Unsupported);
    fs::remove_dir_all(&base).unwrap();
}
