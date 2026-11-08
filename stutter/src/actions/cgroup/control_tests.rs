use std::{fs, path::Path};

use super::*;

fn group(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("stutter-cgctl-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("delegated");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        root.join("cgroup.subtree_control"),
        "cpu cpuset memory io\n",
    )
    .unwrap();
    fs::write(dir.join("cgroup.type"), "domain\n").unwrap();
    fs::write(dir.join("cpu.idle"), "0\n").unwrap();
    fs::write(dir.join("cpuset.cpus.effective"), "0-3\n").unwrap();
    fs::write(dir.join("cpuset.mems.effective"), "0\n").unwrap();
    fs::write(root.join("cpuset.cpus.effective"), "0-3\n").unwrap();
    fs::write(root.join("cpuset.mems.effective"), "0\n").unwrap();
    fs::write(dir.join("io.weight"), "default 100\n").unwrap();
    (root, dir)
}

fn context(dir: &Path) -> CgroupContext {
    CgroupContext::read(dir, crate::syscall::geteuid())
}

#[test]
fn every_control_renders_the_kernel_text_and_reads_back_exactly() {
    let cases: [(CgroupControl, &str, &str); 9] = [
        (CgroupControl::CpuWeight(200), "200", "200\n"),
        (CgroupControl::CpuIdle(true), "1", "1\n"),
        (
            CgroupControl::CpuMax {
                quota_us: Some(50_000),
                period_us: 100_000,
            },
            "50000 100000",
            "50000 100000\n",
        ),
        (
            CgroupControl::CpuMax {
                quota_us: None,
                period_us: 100_000,
            },
            "max 100000",
            "max 100000\n",
        ),
        (CgroupControl::CpusetCpus(vec![0, 1, 2]), "0,1,2", "0-2\n"),
        (CgroupControl::CpusetMems(vec![0]), "0", "0\n"),
        (
            CgroupControl::MemoryMin(1 << 30),
            "1073741824",
            "1073741824\n",
        ),
        (CgroupControl::MemoryHigh(None), "max", "max\n"),
        (CgroupControl::IoWeight(500), "500", "default 500\n"),
    ];
    for (control, rendered, readback) in cases {
        assert_eq!(control.render(), rendered, "{control:?}");
        assert!(control.readback_matches(readback), "{control:?}");
        assert!(!control.readback_matches("garbage"), "{control:?}");
    }
    // A cpuset reads back as a set, whatever order or ranges the kernel prints.
    assert!(CgroupControl::CpusetCpus(vec![2, 0, 1]).readback_matches("0-2"));
    assert!(!CgroupControl::CpusetCpus(vec![0, 1]).readback_matches("0-2"));
}

#[test]
fn io_weight_readback_uses_the_default_row_and_does_not_infer_a_working_device_backend() {
    assert!(CgroupControl::IoWeight(500).readback_matches("default 500\n8:16 200\n8:0 50\n"));
    let (root, dir) = group("io-backend");
    assert_eq!(
        context(&dir).preflight(&CgroupControl::IoWeight(500)),
        Err(ControlDenial::IoWeightMechanismUnknown)
    );
    for bad in [
        "500",
        "default 501",
        "default 500\ndefault 500",
        "default 500\n8:0 200\n8:0 300",
        "default 500\n8:0 default",
        "default 500\n8:0 10001",
        "default 500\ngarbage",
    ] {
        assert!(!CgroupControl::IoWeight(500).readback_matches(bad), "{bad}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unreadable_effective_sets_and_idle_state_do_not_make_controls_eligible() {
    let (root, dir) = group("missing-facts");
    fs::remove_file(root.join("cpuset.cpus.effective")).unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::CpusetCpus(vec![1])),
        Err(ControlDenial::EffectiveSetUnavailable)
    );
    fs::remove_file(dir.join("cpu.idle")).unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::CpuWeight(100)),
        Err(ControlDenial::CpuIdleStateUnknown)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn values_outside_the_kernel_range_are_refused_before_anything_else() {
    for bad in [
        CgroupControl::CpuWeight(0),
        CgroupControl::CpuWeight(10_001),
        CgroupControl::IoWeight(0),
        CgroupControl::CpuMax {
            quota_us: Some(10),
            period_us: 100_000,
        },
        CgroupControl::CpuMax {
            quota_us: Some(50_000),
            period_us: 5,
        },
        CgroupControl::CpusetCpus(vec![]),
        CgroupControl::CpusetCpus(vec![u32::MAX]),
        CgroupControl::CpusetMems(vec![u32::MAX]),
    ] {
        assert!(
            matches!(bad.validate_value(), Err(ControlDenial::OutOfRange(_))),
            "{bad:?}"
        );
    }
    assert_eq!(CgroupControl::CpuWeight(1).validate_value(), Ok(()));
    assert_eq!(CgroupControl::CpuWeight(10_000).validate_value(), Ok(()));
}

#[test]
fn risk_classes_separate_shares_placement_and_hard_limits() {
    assert_eq!(
        CgroupControl::CpuWeight(100).risk(),
        ControlRisk::RelativeShare
    );
    assert_eq!(
        CgroupControl::CpusetCpus(vec![0]).risk(),
        ControlRisk::Placement
    );
    for hard in [
        CgroupControl::CpuMax {
            quota_us: None,
            period_us: 100_000,
        },
        CgroupControl::MemoryMin(1),
        CgroupControl::MemoryHigh(Some(1)),
    ] {
        assert_eq!(hard.risk(), ControlRisk::HardLimit, "{hard:?}");
    }
    assert!(ControlRisk::HardLimit > ControlRisk::RelativeShare);
}

#[test]
fn the_preflight_reads_the_hierarchy_and_denies_each_unsafe_case() {
    let (root, dir) = group("preflight");
    let ctx = context(&dir);
    assert_eq!(ctx.preflight(&CgroupControl::CpuWeight(200)), Ok(()));
    assert_eq!(
        ctx.preflight(&CgroupControl::CpusetCpus(vec![1, 2])),
        Ok(())
    );
    assert_eq!(
        ctx.preflight(&CgroupControl::CpusetCpus(vec![1, 9])),
        Err(ControlDenial::NotInEffectiveSet)
    );
    assert_eq!(
        ctx.preflight(&CgroupControl::IoWeight(300)),
        Err(ControlDenial::IoWeightMechanismUnknown)
    );
    let mut verified_backend = ctx.clone();
    verified_backend.io_weight_honored = Some(true); // Explicit positive device-mechanism fixture.
    assert_eq!(
        verified_backend.preflight(&CgroupControl::IoWeight(300)),
        Ok(())
    );
    // Prospective expansion is bounded by the parent, not by the child's current narrower mask.
    fs::write(dir.join("cpuset.cpus.effective"), "0-1\n").unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::CpusetCpus(vec![2, 3])),
        Ok(())
    );

    // cpu.idle=1 makes cpu.weight meaningless on that group.
    fs::write(dir.join("cpu.idle"), "1\n").unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::CpuWeight(200)),
        Err(ControlDenial::IdleOverridesWeight)
    );
    // A controller the parent does not enable.
    fs::write(root.join("cgroup.subtree_control"), "cpu\n").unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::MemoryHigh(Some(1 << 20))),
        Err(ControlDenial::ControllerNotEnabled("memory"))
    );
    // A threaded group.
    fs::write(
        root.join("cgroup.subtree_control"),
        "cpu cpuset memory io\n",
    )
    .unwrap();
    fs::write(dir.join("cgroup.type"), "threaded\n").unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::CpuWeight(200)),
        Err(ControlDenial::ThreadedGroup)
    );
    // No io.weight file: no device honors it.
    fs::write(dir.join("cgroup.type"), "domain\n").unwrap();
    fs::remove_file(dir.join("io.weight")).unwrap();
    assert_eq!(
        context(&dir).preflight(&CgroupControl::IoWeight(300)),
        Err(ControlDenial::IoWeightNotHonored)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_cgroup_that_is_not_ours_is_not_delegated() {
    let (root, dir) = group("delegation");
    let mut ctx = context(&dir);
    assert!(ctx.delegated);
    ctx.delegated = false;
    assert_eq!(
        ctx.preflight(&CgroupControl::CpuWeight(100)),
        Err(ControlDenial::NotDelegated)
    );
    // Out-of-range wins over delegation, so a malformed request is never mistaken for a policy deny.
    assert!(matches!(
        ctx.preflight(&CgroupControl::CpuWeight(0)),
        Err(ControlDenial::OutOfRange(_))
    ));
    fs::remove_dir_all(&root).unwrap();
}
