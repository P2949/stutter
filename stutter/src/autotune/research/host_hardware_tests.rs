use std::os::unix::fs::PermissionsExt;

use super::*;

struct Fake {
    _root: tempfile::TempDir,
    roots: Roots,
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A twelve-thread CPU (six cores with SMT) and one AMD GPU, shaped like the kernel's views.
fn fake() -> Fake {
    let root = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(root.path()).unwrap();
    let (proc, sys) = (base.join("proc"), base.join("sys"));
    write(&proc.join("sys/kernel/osrelease"), "6.18.57\n");
    write(
        &proc.join("cpuinfo"),
        "processor\t: 0\nmodel name\t: AMD Ryzen 5 7600X 6-Core Processor\nflags\t: x\n",
    );
    write(&sys.join("devices/system/cpu/online"), "0-11\n");
    write(&sys.join("devices/system/cpu/smt/active"), "1\n");
    for cpu in 0..12u32 {
        let sibling = if cpu < 6 { cpu + 6 } else { cpu - 6 };
        let (a, b) = (cpu.min(sibling), cpu.max(sibling));
        write(
            &sys.join(format!(
                "devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"
            )),
            &format!("{a},{b}\n"),
        );
    }
    let card = sys.join("class/drm/card1/device");
    write(&card.join("vendor"), "0x1002\n");
    write(&card.join("device"), "0x744c\n");
    let drivers = sys.join("bus/pci/drivers/amdgpu");
    std::fs::create_dir_all(&drivers).unwrap();
    std::os::unix::fs::symlink(&drivers, card.join("driver")).unwrap();
    // A connector is not a card and is not read.
    write(&sys.join("class/drm/card1-DP-1/status"), "connected\n");
    Fake {
        _root: root,
        roots: Roots { proc, sys },
    }
}

#[test]
fn the_kernel_cpu_topology_and_gpu_are_read_and_a_connector_is_not_a_gpu() {
    let f = fake();
    let hardware = capture(&f.roots).unwrap();
    assert_eq!(hardware.kernel_release, "6.18.57");
    assert_eq!(hardware.cpu_model, "AMD Ryzen 5 7600X 6-Core Processor");
    assert_eq!(hardware.online_cpus, "0-11");
    assert_eq!(hardware.smt_active, Some(true));
    // CPU0 and CPU6 are siblings of one core, not two cores (PLAN P0).
    assert_eq!(hardware.thread_siblings[0], "0,6");
    assert_eq!(hardware.thread_siblings[6], "0,6");
    assert_eq!(hardware.thread_siblings.len(), 12);
    assert_eq!(
        hardware.gpus,
        vec![Gpu {
            vendor: "0x1002".into(),
            device: "0x744c".into(),
            driver: "amdgpu".into()
        }]
    );
}

#[test]
fn each_part_that_changes_is_named_and_the_same_hardware_matches() {
    let changed = |edit: &dyn Fn(&Fake)| {
        let f = fake();
        let before = capture(&f.roots).unwrap();
        edit(&f);
        compare(&before, &capture(&f.roots).unwrap())
    };
    assert_eq!(changed(&|_| {}), Ok(()));
    assert_eq!(
        changed(&|f| write(&f.roots.proc.join("sys/kernel/osrelease"), "6.19.0\n")),
        Err("HostKernelChanged")
    );
    // SMT switched off, a CPU taken offline, and a different CPU model are all topology changes.
    assert_eq!(
        changed(&|f| write(&f.roots.sys.join("devices/system/cpu/smt/active"), "0\n")),
        Err("HostCpuTopologyChanged")
    );
    assert_eq!(
        changed(&|f| write(&f.roots.sys.join("devices/system/cpu/online"), "0-9\n")),
        Err("HostCpuTopologyChanged")
    );
    assert_eq!(
        changed(&|f| write(
            &f.roots.proc.join("cpuinfo"),
            "model name\t: Another Processor\n"
        )),
        Err("HostCpuTopologyChanged")
    );
    // A different GPU device id, and a different bound driver.
    assert_eq!(
        changed(&|f| write(
            &f.roots.sys.join("class/drm/card1/device/device"),
            "0x7480\n"
        )),
        Err("HostGpuChanged")
    );
    assert_eq!(
        changed(&|f| {
            let device = f.roots.sys.join("class/drm/card1/device");
            std::fs::remove_file(device.join("driver")).unwrap();
            let other = f.roots.sys.join("bus/pci/drivers/radeon");
            std::fs::create_dir_all(&other).unwrap();
            std::os::unix::fs::symlink(other, device.join("driver")).unwrap();
        }),
        Err("HostGpuChanged")
    );
    // A second GPU appearing is a change too.
    assert_eq!(
        changed(&|f| {
            let card = f.roots.sys.join("class/drm/card2/device");
            write(&card.join("vendor"), "0x8086\n");
            write(&card.join("device"), "0x56a0\n");
            std::os::unix::fs::symlink(
                f.roots.sys.join("bus/pci/drivers/amdgpu"),
                card.join("driver"),
            )
            .unwrap();
        }),
        Err("HostGpuChanged")
    );
}

#[test]
fn an_unreadable_host_is_never_recorded_or_read_as_unchanged() {
    let f = fake();
    std::fs::remove_file(f.roots.proc.join("sys/kernel/osrelease")).unwrap();
    assert_eq!(capture(&f.roots).unwrap_err(), "HostHardwareUnreadable");
    let g = fake();
    std::fs::remove_dir_all(g.roots.sys.join("devices")).unwrap();
    assert_eq!(capture(&g.roots).unwrap_err(), "HostHardwareUnreadable");
}

#[test]
fn the_baseline_is_taken_once_and_later_hardware_is_compared_with_it() {
    let f = fake();
    let study = tempfile::tempdir().unwrap();
    std::fs::set_permissions(study.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = File::open(study.path()).unwrap();
    let uid = crate::syscall::geteuid();
    assert_eq!(
        status(&f.roots, &dir, uid, &"a".repeat(64)),
        Err("HostHardwareNotRecorded")
    );
    record_baseline(&f.roots, &dir, uid, &"a".repeat(64)).unwrap();
    assert_eq!(status(&f.roots, &dir, uid, &"a".repeat(64)), Ok(()));
    write(&f.roots.proc.join("sys/kernel/osrelease"), "6.19.0\n");
    assert_eq!(
        status(&f.roots, &dir, uid, &"a".repeat(64)),
        Err("HostKernelChanged")
    );
    // A restarted campaign keeps its original baseline instead of re-recording the changed host.
    record_baseline(&f.roots, &dir, uid, &"a".repeat(64)).unwrap();
    assert_eq!(
        status(&f.roots, &dir, uid, &"a".repeat(64)),
        Err("HostKernelChanged")
    );
    // A separate campaign gets its own baseline, without rewriting the first one's evidence.
    assert_eq!(
        status(&f.roots, &dir, uid, &"b".repeat(64)),
        Err("HostHardwareNotRecorded")
    );
    record_baseline(&f.roots, &dir, uid, &"b".repeat(64)).unwrap();
    assert_eq!(status(&f.roots, &dir, uid, &"b".repeat(64)), Ok(()));
    assert_eq!(
        status(&f.roots, &dir, uid, &"a".repeat(64)),
        Err("HostKernelChanged")
    );
}

/// Read-only capture of this machine, printing only the identity (no serials or paths):
/// `cargo test real_host_hardware -- --ignored --nocapture`.
#[test]
#[ignore = "reads this machine's /proc and /sys"]
fn real_host_hardware() {
    let hardware = capture(&Roots::system()).unwrap();
    println!("{hardware:#?}");
    assert_eq!(
        compare(&hardware, &capture(&Roots::system()).unwrap()),
        Ok(())
    );
}
