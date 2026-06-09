use std::{
    cell::Cell,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;

use super::*;
use crate::{
    actions::{
        ActionId, ActionState, ActionWarning, ApplyResult, RollbackToken, SafetyClass,
        TaskIdentity, TuningAction,
        runner::{ActionRunPolicy, run_audited_action_with_audit_path},
    },
    daemon_policy::{ActionSource, DaemonPolicy},
    process_tree::TaskClass,
};

fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid: tid.into(),
        process_pid: Some((tid).into()),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn placement_target(tid: u32, comm: &str, class: TaskClass) -> CgroupPlacementTarget {
    CgroupPlacementTarget {
        identity: target(tid, comm, 12345),
        class,
    }
}

fn action_for(root: &Path, tid: u32) -> CgroupPlacementAction {
    CgroupPlacementAction {
        cgroup_root: root.to_path_buf(),
        target_cgroup: PathBuf::from("/stutter/game.slice"),
        targets: vec![placement_target(tid, "game-thread", TaskClass::Game)],
        cpuset_cpus: Some("2-3".to_owned()),
        cpuset_mems: Some("0".to_owned()),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-cgroup-action-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, starttime_ticks: u64, cgroup: &str) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        task_dir.join("status"),
        format!("Name:\t{comm}\nTgid:\t{tid}\nPid:\t{tid}\n"),
    )
    .unwrap();
    fs::write(
        task_dir.join("stat"),
        fake_stat_line(tid, comm, starttime_ticks),
    )
    .unwrap();
    fs::write(task_dir.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
}

fn fake_stat_line(tid: u32, comm: &str, starttime_ticks: u64) -> String {
    let mut fields = vec!["0".to_owned(); 20];
    fields[0] = "S".to_owned();
    fields[19] = starttime_ticks.to_string();

    format!("{tid} ({comm}) {}", fields.join(" "))
}

fn write_fake_cgroup(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative.trim_start_matches('/'));
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("cgroup.procs"), "").unwrap();
    fs::write(path.join("cpuset.cpus"), "0-7\n").unwrap();
    fs::write(path.join("cpuset.mems"), "0\n").unwrap();
    path
}

struct FailingCgroupWriter {
    writes: usize,
    fail_on_write: usize,
}

impl FailingCgroupWriter {
    fn fail_on_write(fail_on_write: usize) -> Self {
        Self {
            writes: 0,
            fail_on_write,
        }
    }
}

impl CgroupFileWriter for FailingCgroupWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()> {
        self.writes += 1;
        if self.writes == self.fail_on_write {
            anyhow::bail!("injected cgroup write failure for {}", path.display());
        }

        super::write_trimmed(path, value)
    }
}

struct ProcRootCgroupAction<'a> {
    inner: &'a CgroupPlacementAction,
    proc_root: &'a Path,
    rollback_calls: &'a Cell<usize>,
    fail_on_apply_write: usize,
}

impl TuningAction for ProcRootCgroupAction<'_> {
    fn id(&self) -> ActionId {
        self.inner.id()
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.inner.safety_class()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.inner
            .preflight_at(self.proc_root, &CgroupPlacementPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.inner
            .dry_run_at(self.proc_root, &CgroupPlacementPolicy::default())
    }

    fn apply(&self) -> ApplyResult {
        let mut writer = FailingCgroupWriter::fail_on_write(self.fail_on_apply_write);
        self.inner.apply_at_with_writer(
            self.proc_root,
            &CgroupPlacementPolicy::default(),
            &mut writer,
        )
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.inner
            .verify_at(self.proc_root, &CgroupPlacementPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.rollback_calls.set(self.rollback_calls.get() + 1);
        self.inner.rollback_at(self.proc_root, token)
    }
}

mod validation;

mod planning;

mod rollback;

mod procfs;

mod fs_io;
