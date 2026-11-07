use std::fs;

use super::*;
use crate::autotune::operation::MutationOperationKind as Op;

fn root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-owners-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn process(proc_root: &std::path::Path, pid: u32, comm: &str) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    // The kernel truncates comm to 15 bytes.
    let comm: String = comm.chars().take(15).collect();
    fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
}

fn sched_ext_state(sys: &std::path::Path, state: &str, ops: Option<&str>) {
    fs::create_dir_all(sys.join("kernel/sched_ext/root")).unwrap();
    fs::write(sys.join("kernel/sched_ext/state"), format!("{state}\n")).unwrap();
    if let Some(ops) = ops {
        fs::write(sys.join("kernel/sched_ext/root/ops"), format!("{ops}\n")).unwrap();
    }
}

#[test]
fn a_quiet_host_has_no_external_owner() {
    let base = root("quiet");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 10, "bash");
    sched_ext_state(&sys, "disabled", None);
    let report = probe(&proc_root, &sys);
    for domain in ALL_DOMAINS {
        assert_eq!(report.external_owner(domain), None, "{domain:?}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn each_known_owner_claims_exactly_its_own_domains() {
    let cases: [(&str, ResourceDomain, &[ResourceDomain]); 11] = [
        (
            "gamemoded",
            D::TaskPriority,
            &[D::TaskPriority, D::CpuPower],
        ),
        ("lactd", D::GpuPower, &[D::GpuPower]),
        ("ananicy-cpp", D::TaskPriority, &[D::TaskPriority]),
        ("system76-scheduler", D::TaskPriority, &[D::TaskPriority]),
        ("tuned", D::VmKnobs, &[D::CpuPower, D::VmKnobs]),
        ("auto-cpufreq", D::CpuPower, &[D::CpuPower]),
        ("power-profiles-daemon", D::CpuPower, &[D::CpuPower]),
        ("irqbalance", D::IrqAffinity, &[D::IrqAffinity]),
        ("bpftune", D::Network, &[D::Network, D::VmKnobs]),
        ("numad", D::MemoryPlacement, &[D::MemoryPlacement]),
        ("scx_loader", D::Scheduler, &[D::Scheduler]),
    ];
    for (comm, sample, claimed) in cases {
        let base = root(comm);
        let (proc_root, sys) = (base.join("proc"), base.join("sys"));
        fs::create_dir_all(&proc_root).unwrap();
        process(&proc_root, 11, comm);
        sched_ext_state(&sys, "disabled", None);
        let report = probe(&proc_root, &sys);
        assert!(report.external_owner(sample).is_some(), "{comm}");
        for domain in ALL_DOMAINS {
            assert_eq!(
                report.external_owner(domain).is_some(),
                claimed.contains(&domain),
                "{comm} / {domain:?}"
            );
        }
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn a_loaded_scheduler_owns_the_scheduler_even_without_a_manager() {
    let base = root("scx");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    sched_ext_state(&sys, "enabled", Some("scx_lavd"));
    let report = probe(&proc_root, &sys);
    assert_eq!(
        report.sched_ext,
        SchedExt::Enabled {
            ops: "scx_lavd".into()
        }
    );
    assert!(report.external_owner(D::Scheduler).is_some());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_oom_guards_are_reported_but_own_no_tuning_domain() {
    let base = root("oomd");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    process(&proc_root, 12, "systemd-oomd");
    process(&proc_root, 13, "uresourced");
    let report = probe(&proc_root, &sys);
    assert_eq!(report.daemon(KnownOwner::SystemdOomd), Daemon::Running);
    assert_eq!(report.daemon(KnownOwner::Uresourced), Daemon::Running);
    for domain in ALL_DOMAINS {
        assert_eq!(report.external_owner(domain), None, "{domain:?}");
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn unreadable_state_fails_closed() {
    let base = root("unknown");
    let sys = base.join("sys");
    sched_ext_state(&sys, "garbage", None);
    let report = probe(&base.join("missing-proc"), &sys);
    assert_eq!(report.sched_ext, SchedExt::Unknown);
    assert_eq!(report.daemon(KnownOwner::GameMode), Daemon::Unknown);
    for domain in ALL_DOMAINS {
        // Domains with no possible owner stay free; every claimed one is owned.
        let claimed =
            domain == D::Scheduler || OWNERS.iter().any(|spec| spec.domains.contains(&domain));
        assert_eq!(
            report.external_owner(domain).is_some(),
            claimed,
            "{domain:?}"
        );
    }
    let none = probe(&base.join("missing-proc"), &base.join("no-sys"));
    assert_eq!(none.sched_ext, SchedExt::Unsupported);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_owned_domain_denies_only_the_operations_that_change_it() {
    let base = root("deny");
    let (proc_root, sys) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    for (pid, comm) in [
        (1, "gamemoded"),
        (2, "lactd"),
        (3, "irqbalance"),
        (4, "tuned"),
    ] {
        process(&proc_root, pid, comm);
    }
    let owned = probe(&proc_root, &sys);
    for op in [
        Op::Nice,
        Op::IoPrio,
        Op::CpuPowerEpp,
        Op::CpuPowerGovernor,
        Op::GpuPowerProfile,
        Op::GpuForcePerformance,
        Op::VmSwappiness,
        Op::VmDirtyRatio,
        Op::IrqKnownDevice,
    ] {
        assert!(owner_denial(Some(&owned), op).is_some(), "{op:?}");
    }
    // The Supported tuples are never blocked by another owner.
    for op in [Op::CpuAffinityProfile, Op::CgroupPlacement] {
        assert_eq!(owner_denial(Some(&owned), op), None, "{op:?}");
    }
    assert_eq!(owner_denial(None, Op::Nice), None);
    fs::remove_dir_all(&base).unwrap();
}
