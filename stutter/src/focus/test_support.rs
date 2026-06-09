//! Test-only builders shared by the focus module test split.
//!
//! Owns synthetic focus snapshots, process builders, resolver fixtures, and fake procfs helpers.
//! Does not own test assertions or production focus behavior.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::*;
use crate::foreground::{ForegroundAvailableInput, ForegroundSource, ForegroundWindowSnapshot};

pub(crate) fn foreground_snapshot(pid: Option<u32>) -> ForegroundWindowSnapshot {
    ForegroundWindowSnapshot::available(ForegroundAvailableInput {
        elapsed_ms: 1_000,
        source: ForegroundSource::Sway,
        pid,
        app_id: Some("steam".to_owned()),
        class: Some("Steam".to_owned()),
        title: None,
        include_title: false,
        window_id: Some("7".to_owned()),
        workspace: Some("games".to_owned()),
        confidence: 0.95,
        reason: "test foreground snapshot".to_owned(),
    })
}

fn foreground_test_classification(
    class: SystemTaskClass,
    priority_band: PriorityBand,
    confidence: f32,
) -> Classification {
    Classification {
        class,
        priority_band,
        confidence,
        reasons: vec![format!("test class {:?}", class)],
    }
}

pub(crate) fn foreground_test_process(pid: u32, ppid: u32, class: SystemTaskClass) -> FocusProcess {
    FocusProcess {
        pid,
        ppid,
        comm: format!("process-{pid}"),
        cmdline: format!("process-{pid}"),
        cgroup_path: None,
        starttime_ticks: Some(pid as u64 * 10),
        sched_policy: None,
        is_foreground_window_process: false,
        classification: foreground_test_classification(class, PriorityBand::Interactive, 0.85),
        cpu_time_ticks_delta: 10,
        read_bytes_delta: 0,
        write_bytes_delta: 0,
        voluntary_ctxt_switches_delta: 0,
        nonvoluntary_ctxt_switches_delta: 0,
    }
}

pub(crate) fn foreground_test_group(
    kind: FocusGroupKind,
    root_pids: Vec<u32>,
    member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    display_name: &str,
    score: f32,
) -> FocusGroup {
    FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name: display_name.to_owned(),
        score,
        score_breakdown: FocusScoreBreakdown {
            cpu_score: 0.50,
            io_score: 0.10,
            interactivity_score: 0.50,
            class_priority_score: 0.50,
            stability_score: 0.50,
            foreground_score: 0.0,
            penalty: 0.0,
        },
        confidence: 0.80,
        priority_band: PriorityBand::Interactive,
        reasons: vec![format!("test group {display_name}")],
    }
}

pub(crate) fn foreground_scoring_snapshot(
    foreground: Option<ForegroundWindowSnapshot>,
    groups: Vec<FocusGroup>,
) -> FocusSnapshot {
    let mut processes = BTreeMap::new();
    processes.insert(10, foreground_test_process(10, 1, SystemTaskClass::Game));
    processes.insert(
        11,
        foreground_test_process(11, 10, SystemTaskClass::GameWorkerThread),
    );
    processes.insert(
        20,
        foreground_test_process(20, 1, SystemTaskClass::BrowserForeground),
    );
    processes.insert(
        21,
        foreground_test_process(21, 20, SystemTaskClass::BrowserRenderer),
    );
    processes.insert(
        30,
        foreground_test_process(30, 1, SystemTaskClass::Compiler),
    );

    let mut children_by_parent = BTreeMap::new();
    children_by_parent.insert(1, vec![10, 20, 30]);
    children_by_parent.insert(10, vec![11]);
    children_by_parent.insert(20, vec![21]);

    FocusSnapshot {
        elapsed_ms: 1_000,
        foreground,
        processes,
        children_by_parent,
        groups,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FakeProcProcess {
    pub(crate) pid: u32,
    pub(crate) ppid: u32,
    pub(crate) comm: String,
    pub(crate) cmdline: String,
    pub(crate) cgroup_path: String,
    pub(crate) sched_policy: Option<u32>,
    pub(crate) starttime_ticks: u64,
    pub(crate) cpu_time_ticks: u64,
    pub(crate) read_bytes: u64,
    pub(crate) write_bytes: u64,
    pub(crate) voluntary_ctxt_switches: u64,
    pub(crate) nonvoluntary_ctxt_switches: u64,
}

impl FakeProcProcess {
    pub(crate) fn new(pid: u32, ppid: u32, comm: &str, cmdline: &str) -> Self {
        Self {
            pid,
            ppid,
            comm: comm.to_owned(),
            cmdline: cmdline.to_owned(),
            cgroup_path: format!("/user.slice/test-{}.scope", pid),
            sched_policy: None,
            starttime_ticks: pid as u64 * 100,
            cpu_time_ticks: 0,
            read_bytes: 0,
            write_bytes: 0,
            voluntary_ctxt_switches: 0,
            nonvoluntary_ctxt_switches: 0,
        }
    }

    pub(crate) fn with_cgroup(mut self, cgroup_path: &str) -> Self {
        self.cgroup_path = cgroup_path.to_owned();
        self
    }

    pub(crate) fn with_sched_policy(mut self, sched_policy: u32) -> Self {
        self.sched_policy = Some(sched_policy);
        self
    }

    pub(crate) fn with_activity(
        mut self,
        cpu_time_ticks: u64,
        read_bytes: u64,
        write_bytes: u64,
        voluntary_ctxt_switches: u64,
        nonvoluntary_ctxt_switches: u64,
    ) -> Self {
        self.cpu_time_ticks = cpu_time_ticks;
        self.read_bytes = read_bytes;
        self.write_bytes = write_bytes;
        self.voluntary_ctxt_switches = voluntary_ctxt_switches;
        self.nonvoluntary_ctxt_switches = nonvoluntary_ctxt_switches;
        self
    }
}

fn focus_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-focus-test-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_proc_process(proc_root: &Path, process: &FakeProcProcess) {
    let process_dir = proc_root.join(process.pid.to_string());
    std::fs::create_dir_all(&process_dir).unwrap();

    std::fs::write(
        process_dir.join("status"),
        format!(
            "Name:\t{}\nPPid:\t{}\nvoluntary_ctxt_switches:\t{}\nnonvoluntary_ctxt_switches:\t{}\n",
            process.comm,
            process.ppid,
            process.voluntary_ctxt_switches,
            process.nonvoluntary_ctxt_switches
        ),
    )
    .unwrap();

    std::fs::write(
        process_dir.join("cmdline"),
        process
            .cmdline
            .split(' ')
            .collect::<Vec<_>>()
            .join("\0")
            .into_bytes(),
    )
    .unwrap();

    std::fs::write(
        process_dir.join("cgroup"),
        format!("0::{}\n", process.cgroup_path),
    )
    .unwrap();

    std::fs::write(
        process_dir.join("io"),
        format!(
            "read_bytes: {}\nwrite_bytes: {}\n",
            process.read_bytes, process.write_bytes
        ),
    )
    .unwrap();

    std::fs::write(process_dir.join("stat"), fake_stat_line(process)).unwrap();
}

fn fake_stat_line(process: &FakeProcProcess) -> String {
    let mut fields_after_comm = vec![
        "S".to_owned(),
        process.ppid.to_string(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        process.cpu_time_ticks.to_string(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "20".to_owned(),
        "0".to_owned(),
        "1".to_owned(),
        "0".to_owned(),
        process.starttime_ticks.to_string(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        "0".to_owned(),
        process.sched_policy.unwrap_or(0).to_string(),
    ];

    while fields_after_comm.len() < 42 {
        fields_after_comm.push("0".to_owned());
    }

    format!(
        "{} ({}) {}",
        process.pid,
        process.comm,
        fields_after_comm.join(" ")
    )
}

pub(crate) fn focus_snapshot_from_fake_proc(
    test_name: &str,
    first_sample: Vec<FakeProcProcess>,
    second_sample: Vec<FakeProcProcess>,
) -> FocusSnapshot {
    let proc_root = focus_temp_dir(test_name);
    let mut cache = FocusCache::default();

    for process in &first_sample {
        write_fake_proc_process(&proc_root, process);
    }

    let _ = focus_snapshot_at(&proc_root, &mut cache, 0, None);

    for process in &second_sample {
        write_fake_proc_process(&proc_root, process);
    }

    let snapshot = focus_snapshot_at(&proc_root, &mut cache, 1000, None);
    std::fs::remove_dir_all(proc_root).ok();
    snapshot
}

fn required_test_classification(
    class: SystemTaskClass,
    priority_band: PriorityBand,
    confidence: f32,
) -> Classification {
    Classification {
        class,
        priority_band,
        confidence,
        reasons: vec![format!("required focus test classification {class:?}")],
    }
}

pub(crate) fn required_test_process(
    pid: u32,
    ppid: u32,
    comm: &str,
    class: SystemTaskClass,
    priority_band: PriorityBand,
    cpu_time_ticks_delta: u64,
) -> FocusProcess {
    FocusProcess {
        pid,
        ppid,
        comm: comm.to_owned(),
        cmdline: comm.to_owned(),
        cgroup_path: None,
        starttime_ticks: Some(pid as u64 * 100),
        sched_policy: None,
        is_foreground_window_process: false,
        classification: required_test_classification(class, priority_band, 0.85),
        cpu_time_ticks_delta,
        read_bytes_delta: 0,
        write_bytes_delta: 0,
        voluntary_ctxt_switches_delta: 0,
        nonvoluntary_ctxt_switches_delta: 0,
    }
}

pub(crate) fn required_test_group(
    kind: FocusGroupKind,
    root_pids: Vec<u32>,
    member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    score: f32,
    confidence: f32,
) -> FocusGroup {
    FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name: format!("{kind:?}"),
        score,
        score_breakdown: FocusScoreBreakdown::default(),
        confidence,
        priority_band: match kind {
            FocusGroupKind::Game | FocusGroupKind::Browser | FocusGroupKind::Desktop => {
                PriorityBand::ForegroundLatency
            }
            FocusGroupKind::Compile => PriorityBand::Throughput,
            FocusGroupKind::Idle => PriorityBand::Background,
            FocusGroupKind::Media
            | FocusGroupKind::Recording
            | FocusGroupKind::VirtualMachine
            | FocusGroupKind::Unknown => PriorityBand::Interactive,
        },
        reasons: vec![format!("required focus test group {kind:?}")],
    }
}

pub(crate) fn required_test_snapshot(
    groups: Vec<FocusGroup>,
    processes: Vec<FocusProcess>,
    elapsed_ms: u64,
) -> FocusSnapshot {
    let mut process_map = BTreeMap::new();
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    for process in processes {
        children_by_parent
            .entry(process.ppid)
            .or_default()
            .push(process.pid);
        process_map.insert(process.pid, process);
    }

    for children in children_by_parent.values_mut() {
        children.sort_unstable();
    }

    FocusSnapshot {
        elapsed_ms,
        foreground: None,
        processes: process_map,
        children_by_parent,
        groups,
    }
}

pub(crate) fn required_focus_policy() -> FocusPolicy {
    FocusPolicy {
        poll_ms: 1000,
        min_confidence: 0.60,
        switch_margin: 0.20,
        switch_cooldown_ms: 0,
        required_winner_polls: 2,
        max_roots: 4,
    }
}

pub(crate) fn first_focus_group(snapshot: &FocusSnapshot, kind: FocusGroupKind) -> &FocusGroup {
    snapshot
        .groups
        .iter()
        .find(|group| group.kind == kind)
        .unwrap()
}

pub(crate) fn situation_mapping_test_group(kind: FocusGroupKind) -> FocusGroup {
    FocusGroup {
        kind,
        root_pids: vec![1],
        member_pids: vec![1],
        primary_pid: Some(1),
        display_name: format!("{kind:?}"),
        score: 0.75,
        score_breakdown: FocusScoreBreakdown::default(),
        confidence: 0.75,
        priority_band: PriorityBand::Interactive,
        reasons: vec![format!("situation mapping test {kind:?}")],
    }
}

fn resolver_test_classification(
    class: SystemTaskClass,
    priority_band: PriorityBand,
    confidence: f32,
) -> Classification {
    Classification {
        class,
        priority_band,
        confidence,
        reasons: vec![format!("resolver test classification {class:?}")],
    }
}

pub(crate) fn resolver_test_process(pid: u32, comm: &str, class: SystemTaskClass) -> FocusProcess {
    FocusProcess {
        pid,
        ppid: 1,
        comm: comm.to_owned(),
        cmdline: comm.to_owned(),
        cgroup_path: None,
        starttime_ticks: Some(pid as u64 * 100),
        sched_policy: None,
        is_foreground_window_process: false,
        classification: resolver_test_classification(class, PriorityBand::Interactive, 0.80),
        cpu_time_ticks_delta: 10,
        read_bytes_delta: 0,
        write_bytes_delta: 0,
        voluntary_ctxt_switches_delta: 0,
        nonvoluntary_ctxt_switches_delta: 0,
    }
}

pub(crate) fn resolver_test_group(
    kind: FocusGroupKind,
    root_pids: Vec<u32>,
    member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    score: f32,
    confidence: f32,
) -> FocusGroup {
    FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name: format!("{kind:?}"),
        score,
        score_breakdown: FocusScoreBreakdown::default(),
        confidence,
        priority_band: match kind {
            FocusGroupKind::Game | FocusGroupKind::Browser | FocusGroupKind::Desktop => {
                PriorityBand::ForegroundLatency
            }
            FocusGroupKind::Compile => PriorityBand::Throughput,
            FocusGroupKind::Idle => PriorityBand::Background,
            FocusGroupKind::Media
            | FocusGroupKind::Recording
            | FocusGroupKind::VirtualMachine
            | FocusGroupKind::Unknown => PriorityBand::Interactive,
        },
        reasons: vec![format!("resolver test group {kind:?}")],
    }
}

pub(crate) fn resolver_test_snapshot(
    groups: Vec<FocusGroup>,
    processes: Vec<FocusProcess>,
    elapsed_ms: u64,
) -> FocusSnapshot {
    let mut process_map = BTreeMap::new();
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    for process in processes {
        children_by_parent
            .entry(process.ppid)
            .or_default()
            .push(process.pid);
        process_map.insert(process.pid, process);
    }

    for children in children_by_parent.values_mut() {
        children.sort_unstable();
    }

    FocusSnapshot {
        elapsed_ms,
        foreground: None,
        processes: process_map,
        children_by_parent,
        groups,
    }
}

pub(crate) fn resolver_test_policy() -> FocusPolicy {
    FocusPolicy {
        poll_ms: 1000,
        min_confidence: 0.60,
        switch_margin: 0.20,
        switch_cooldown_ms: 5000,
        required_winner_polls: 2,
        max_roots: 4,
    }
}

fn group_test_classification(
    class: SystemTaskClass,
    priority_band: PriorityBand,
    confidence: f32,
) -> Classification {
    Classification {
        class,
        priority_band,
        confidence,
        reasons: vec![format!("test classification {class:?}")],
    }
}

pub(crate) fn group_test_process(
    pid: u32,
    ppid: u32,
    comm: &str,
    class: SystemTaskClass,
    priority_band: PriorityBand,
    cpu_time_ticks_delta: u64,
) -> FocusProcess {
    FocusProcess {
        pid,
        ppid,
        comm: comm.to_owned(),
        cmdline: comm.to_owned(),
        cgroup_path: None,
        starttime_ticks: Some(pid as u64 * 10),
        sched_policy: None,
        is_foreground_window_process: false,
        classification: group_test_classification(class, priority_band, 0.9),
        cpu_time_ticks_delta,
        read_bytes_delta: 0,
        write_bytes_delta: 0,
        voluntary_ctxt_switches_delta: 0,
        nonvoluntary_ctxt_switches_delta: 0,
    }
}

pub(crate) fn group_test_snapshot(processes: Vec<FocusProcess>) -> FocusSnapshot {
    let mut process_map = BTreeMap::new();
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    for process in processes {
        children_by_parent
            .entry(process.ppid)
            .or_default()
            .push(process.pid);
        process_map.insert(process.pid, process);
    }

    for children in children_by_parent.values_mut() {
        children.sort_unstable();
    }

    FocusSnapshot {
        elapsed_ms: 1000,
        foreground: None,
        processes: process_map,
        children_by_parent,
        groups: Vec::new(),
    }
}
