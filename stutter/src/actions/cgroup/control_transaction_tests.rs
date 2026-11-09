use std::{cell::RefCell, collections::BTreeMap, fs};

use super::*;

fn group(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-cgtx-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("delegated");
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in [
        ("cgroup.type", "domain\n"),
        ("cpu.idle", "0\n"),
        ("cpu.weight", "100\n"),
        ("cpu.max", "max 100000\n"),
        ("cpuset.cpus", "0-3\n"),
        ("cpuset.mems", "0\n"),
        ("memory.min", "0\n"),
        ("memory.low", "0\n"),
        ("memory.high", "max\n"),
        ("io.weight", "default 100\n"),
        ("cpuset.cpus.effective", "0-3\n"),
        ("cpuset.mems.effective", "0\n"),
    ] {
        fs::write(dir.join(file), text).unwrap();
    }
    fs::write(
        root.join("cgroup.subtree_control"),
        "cpu cpuset memory io\n",
    )
    .unwrap();
    fs::write(root.join("cpuset.cpus.effective"), "0-3\n").unwrap();
    fs::write(root.join("cpuset.mems.effective"), "0\n").unwrap();
    dir
}

fn euid() -> u32 {
    crate::syscall::geteuid()
}

fn read(dir: &Path, file: &str) -> String {
    fs::read_to_string(dir.join(file))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn every_control_prepares_commits_and_restores_distinctly() {
    let cases = [
        (CgroupControl::CpuWeight(300), "cpu.weight", "300", "100"),
        (CgroupControl::CpuIdle(true), "cpu.idle", "1", "0"),
        (
            CgroupControl::CpuMax {
                quota_us: Some(50_000),
                period_us: 100_000,
            },
            "cpu.max",
            "50000 100000",
            "max 100000",
        ),
        (
            CgroupControl::CpusetCpus(vec![0, 1]),
            "cpuset.cpus",
            "0,1",
            "0-3",
        ),
        (CgroupControl::CpusetMems(vec![0]), "cpuset.mems", "0", "0"),
        (
            CgroupControl::MemoryMin(1 << 20),
            "memory.min",
            "1048576",
            "0",
        ),
        (
            CgroupControl::MemoryLow(1 << 20),
            "memory.low",
            "1048576",
            "0",
        ),
        (
            CgroupControl::MemoryHigh(Some(1 << 30)),
            "memory.high",
            "1073741824",
            "max",
        ),
        (
            CgroupControl::IoWeight(500),
            "io.weight",
            "500",
            "default 100",
        ),
    ];
    for (control, file, applied, original) in cases {
        let dir = group(file);
        let prepared = match prepare(&dir, &control, euid(), Some(true), &Kernelish) {
            Ok(prepared) => prepared,
            // cpuset.mems 0 already equals the baseline: a no-op is refused, not committed.
            Err(PrepareError::NoChange) => {
                assert_eq!(file, "cpuset.mems");
                continue;
            }
            Err(error) => panic!("{file}: {error:?}"),
        };
        assert_eq!(read(&dir, file), original, "{file}: prepare must not write");
        let committed = prepared.commit(&Kernelish).unwrap();
        assert!(
            control.readback_matches(&read(&dir, file)),
            "{file} applied as {applied}"
        );
        assert_eq!(
            committed.restore(&Kernelish),
            RestoreOutcome::Restored,
            "{file}"
        );
        assert_eq!(
            read(&dir, file),
            if file == "io.weight" {
                "default 100"
            } else {
                original
            },
            "{file}"
        );
    }
}

#[test]
fn preflight_denials_and_unrestorable_baselines_are_refused_before_any_write() {
    let dir = group("deny");
    assert_eq!(
        prepare(
            &dir,
            &CgroupControl::CpuWeight(0),
            euid(),
            Some(true),
            &RealFiles
        )
        .unwrap_err(),
        PrepareError::Denied(ControlDenial::OutOfRange("weight must be 1..=10000"))
    );
    assert_eq!(
        prepare(
            &dir,
            &CgroupControl::CpusetCpus(vec![9]),
            euid(),
            Some(true),
            &RealFiles
        )
        .unwrap_err(),
        PrepareError::Denied(ControlDenial::NotInEffectiveSet)
    );
    assert_eq!(
        prepare(
            &dir,
            &CgroupControl::CpuWeight(100),
            euid(),
            Some(true),
            &RealFiles
        )
        .unwrap_err(),
        PrepareError::NoChange
    );
    fs::write(dir.join("io.weight"), "default 100\n8:0 200\n").unwrap();
    assert_eq!(
        prepare(
            &dir,
            &CgroupControl::IoWeight(500),
            euid(),
            Some(true),
            &RealFiles
        )
        .unwrap_err(),
        PrepareError::UnsupportedRestore
    );
    fs::remove_file(dir.join("cpu.max")).unwrap();
    let max = CgroupControl::CpuMax {
        quota_us: Some(1000),
        period_us: 100_000,
    };
    assert_eq!(
        prepare(&dir, &max, euid(), Some(true), &RealFiles).unwrap_err(),
        PrepareError::Unavailable
    );
    fs::write(dir.join("cgroup.type"), "domain threaded\n").unwrap();
    assert_eq!(
        prepare(
            &dir,
            &CgroupControl::MemoryLow(4096),
            euid(),
            Some(true),
            &RealFiles
        )
        .unwrap_err(),
        PrepareError::Denied(ControlDenial::ThreadedGroup)
    );
}

#[test]
fn a_foreign_change_between_prepare_and_commit_aborts_without_writing() {
    let dir = group("race");
    let prepared = prepare(
        &dir,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &RealFiles,
    )
    .unwrap();
    fs::write(dir.join("cpu.weight"), "777\n").unwrap();
    assert_eq!(
        prepared.commit(&RealFiles).unwrap_err(),
        CommitError::OwnerChanged
    );
    assert_eq!(read(&dir, "cpu.weight"), "777");
}

#[test]
fn a_replaced_group_is_never_written_or_restored() {
    let dir = group("replaced");
    let prepared = prepare(
        &dir,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &RealFiles,
    )
    .unwrap();
    let held = group("replaced-twin");
    fs::remove_dir_all(&dir).unwrap();
    fs::rename(&held, &dir).unwrap();
    assert_eq!(
        prepared.commit(&RealFiles).unwrap_err(),
        CommitError::GroupChanged
    );

    let dir = group("replaced-after");
    let committed = prepare(
        &dir,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &RealFiles,
    )
    .unwrap()
    .commit(&RealFiles)
    .unwrap();
    let twin = group("replaced-after-twin");
    fs::remove_dir_all(&dir).unwrap();
    fs::rename(&twin, &dir).unwrap();
    assert_eq!(
        committed.restore(&RealFiles),
        RestoreOutcome::UnsafeToRestore
    );
    assert_eq!(read(&dir, "cpu.weight"), "100");
}

#[test]
fn restore_leaves_a_value_another_owner_set_after_ours() {
    let dir = group("takeover");
    let committed = prepare(
        &dir,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &RealFiles,
    )
    .unwrap()
    .commit(&RealFiles)
    .unwrap();
    fs::write(dir.join("cpu.weight"), "555\n").unwrap();
    assert_eq!(
        committed.restore(&RealFiles),
        RestoreOutcome::ExternallyChanged
    );
    assert_eq!(read(&dir, "cpu.weight"), "555");
    let again = group("already");
    let committed = prepare(
        &again,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &RealFiles,
    )
    .unwrap()
    .commit(&RealFiles)
    .unwrap();
    fs::write(again.join("cpu.weight"), "100\n").unwrap();
    assert_eq!(
        committed.restore(&RealFiles),
        RestoreOutcome::AlreadyRestored
    );
}

/// A kernel stand-in that clamps what is written, so the readback differs from the request.
struct Clamping(RefCell<BTreeMap<PathBuf, String>>);

impl ControlFiles for Clamping {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.0
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::other("absent"))
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        let stored = if text == "300" {
            "100".to_owned()
        } else {
            text.to_owned()
        };
        self.0.borrow_mut().insert(path.to_owned(), stored);
        Ok(())
    }
}

#[test]
fn a_value_the_kernel_did_not_apply_is_written_back_and_reported() {
    let dir = group("clamp");
    let files = Clamping(RefCell::new(BTreeMap::from([(
        dir.join("cpu.weight"),
        "100\n".to_owned(),
    )])));
    let prepared = prepare(
        &dir,
        &CgroupControl::CpuWeight(300),
        euid(),
        Some(true),
        &files,
    )
    .unwrap();
    assert_eq!(
        prepared.commit(&files).unwrap_err(),
        CommitError::ReadbackMismatch { restored: true }
    );
    assert_eq!(files.read(&dir.join("cpu.weight")).unwrap().trim(), "100");
}

/// Real files, with the one kernel behaviour a plain file lacks: `io.weight` reads back as
/// `default N` after a bare weight is written.
struct Kernelish;

impl ControlFiles for Kernelish {
    fn read(&self, path: &Path) -> io::Result<String> {
        RealFiles.read(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        if path.ends_with("io.weight") && !text.contains(' ') {
            return RealFiles.write(path, &format!("default {text}\n"));
        }
        RealFiles.write(path, text)
    }
}
