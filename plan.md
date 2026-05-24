# Report: remaining raw task/process ID boundaries in `stutter-experimental(2).zip`

## Implementation status

- [x] 1. Add `Default` to typed numeric IDs in `stutter-core/src/ids.rs`.
- [x] 2. Migrate `focus/classify.rs` process/thread identities to `Pid` / `Tid`.
- [x] 3. Migrate `affinity.rs` restore records and add typed affinity wrappers.
- [x] 4. Migrate `profile_restore.rs` persisted restore records to `Pid` / `Tid`.
- [x] 5. Migrate `profiles.rs` task plans/cache keys and active-config lookup conversions.
- [x] 6. Migrate `autotune/washout.rs` test-only identities.
- [x] 7. Migrate `recorder/event_types.rs` artifact DTOs and call sites.
- [x] 8. Add serde compatibility and architecture regression tests.
- [x] 9. Run formatting, tests, and clippy.

I checked the uploaded tree directly. The issue is real, but the fixes should be split by boundary type. **Internal models should move to `Tid` / `Pid`; persisted JSON DTOs can also move to `Tid` / `Pid` because the existing ID wrappers use `#[serde(transparent)]`; eBPF ABI structs must stay raw `u32`.**

The existing typed-ID foundation is already present in `stutter-core/src/ids.rs`: the numeric ID macro derives serde and uses `#[serde(transparent)]`, and `Pid` / `Tid` are declared from that macro at lines 8-12 and 131-132. That means JSON shape can stay numeric while Rust fields become typed.

---

## Foundation change needed first: give typed numeric IDs `Default`

Several recorder DTOs derive `Default`. Replacing `u32` with `Pid` / `Tid` will fail unless those wrappers also default to `0`, which matches old `u32::default()` behavior.

### Current code

`stutter-core/src/ids.rs:8-12`:

```rust
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct $name(u32);
```

### Proposed code

```rust
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct $name(u32);
```

This is safe because `Pid::default()` and `Tid::default()` become `Pid(0)` / `Tid(0)`, which preserves the old DTO default behavior.

---

# Issue 1: `focus/classify.rs` still exposes raw process/thread IDs

## Current problematic code

`stutter/src/focus/classify.rs:29-48`:

```rust
#[derive(Debug, Clone)]
pub struct ProcessIdentity<'a> {
    pub pid: u32,
    pub ppid: u32,
    pub comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: Option<&'a str>,
    pub cgroup_path: Option<&'a str>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ThreadIdentity<'a> {
    pub tid: u32,
    pub process_pid: u32,
    pub process_class: SystemTaskClass,
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub sched_policy: Option<u32>,
}
```

This is not serialized, so there is no compatibility blocker. It is also publicly re-exported through `stutter/src/focus/public_api.rs:13-17`, so it is a real API boundary, not just a private helper.

There are also raw comparisons using those fields, for example `identity.ppid == 2` at `stutter/src/focus/classify.rs:66`, `identity.pid != 1` at line 241, and `identity.pid == 1` at line 260.

## Fix plan

1. Import `Pid` / `Tid`.
2. Change `ProcessIdentity.pid` and `ProcessIdentity.ppid` to `Pid`.
3. Change `ThreadIdentity.tid` to `Tid` and `ThreadIdentity.process_pid` to `Pid`.
4. Update direct numeric comparisons to compare against `Pid::new(...)`.
5. Update call sites to pass typed IDs or use `.into()` at raw procfs boundaries.
6. Update focus tests.

## Proposed code

```rust
use serde::{Deserialize, Serialize};
use stutter_core::ids::{Pid, Tid};

use super::community_rules::try_community_rules_classification;
use crate::{ascii_match::AsciiCase, process_tree::TaskClass as SystemTaskClass};

#[derive(Debug, Clone)]
pub struct ProcessIdentity<'a> {
    pub pid: Pid,
    pub ppid: Pid,
    pub comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: Option<&'a str>,
    pub cgroup_path: Option<&'a str>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ThreadIdentity<'a> {
    pub tid: Tid,
    pub process_pid: Pid,
    pub process_class: SystemTaskClass,
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub sched_policy: Option<u32>,
}
```

Then update comparisons:

```rust
} else if identity.ppid == Pid::new(2)
    || comm.starts_with("kworker")
    || comm.starts_with("ksoftirqd")
{
    // ...
} else if identity.pid != Pid::new(1)
    && !cgroup_path_fold.contains(".service")
    && !cgroup_path_fold.contains("/system.slice/")
    && let Some(res) = try_community_rules_classification(&mut reasons, identity, cgroup_path)
{
    res
} else if identity.pid == Pid::new(1)
    || cgroup_path_fold.contains(".service")
    || cgroup_path_fold.contains("/system.slice/")
{
    // ...
}
```

Representative call-site fix in `stutter/src/focus/snapshot.rs:99-107`:

```rust
let classification = classify_process(&ProcessIdentity {
    pid: proc_info.pid.into(),
    ppid: proc_info.ppid.into(),
    comm: &proc_info.comm,
    cmdline: &proc_info.cmdline,
    exe_path,
    cgroup_path,
    sched_policy: proc_info.sched_policy,
});
```

Test constructors should become:

```rust
let classification = classify_process(&ProcessIdentity {
    pid: 1400.into(),
    ppid: 1.into(),
    comm: "pipewire",
    cmdline: "pipewire",
    exe_path: None,
    cgroup_path: None,
    sched_policy: Some(SCHED_FIFO),
});
```

---

# Issue 2: `affinity.rs::AffinityRecord` is a persisted restore record with raw IDs

## Current problematic code

`stutter/src/affinity.rs:19-30`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffinityRecord {
    pub tid: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    pub original_mask: CpuMask,
    pub applied_mask: CpuMask,
}
```

The restore validation path also stays raw:

`stutter/src/affinity.rs:333-352`:

```rust
fn restore_record_status_at(
    proc_root: &Path,
    record: &AffinityRecord,
) -> io::Result<RestoreRecordStatus> {
    restore_identity_status_at(
        proc_root,
        record.tid,
        record.process_pid,
        record.process_starttime_ticks,
        record.task_starttime_ticks,
    )
}

pub(crate) fn restore_identity_status_at(
    proc_root: &Path,
    tid: u32,
    process_pid: Option<u32>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
```

There is also a raw merge key at `stutter/src/affinity.rs:504-521`.

## Fix plan

1. Import `Pid` / `Tid`.
2. Change `AffinityRecord.tid` to `Tid`.
3. Change `AffinityRecord.process_pid` to `Option<Pid>`.
4. Change restore-status helpers to accept typed IDs.
5. Change `RestoreMergeKey` to use typed IDs.
6. Keep `read_allowed_mask_raw(tid: u32)` and `set_affinity_raw(tid: u32, ...)` as raw syscall boundaries, but add typed wrappers for non-syscall code.
7. Add JSON compatibility tests proving `Tid` / `Pid` still serialize as numbers.

## Proposed code

```rust
use stutter_core::ids::{Pid, Tid};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffinityRecord {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    pub original_mask: CpuMask,
    pub applied_mask: CpuMask,
}
```

Add typed wrappers while preserving raw syscall functions:

```rust
pub fn read_allowed_mask(tid: Tid) -> io::Result<CpuMask> {
    read_allowed_mask_raw(tid.as_u32())
}

pub fn set_affinity(tid: Tid, mask: &CpuMask) -> io::Result<()> {
    set_affinity_raw(tid.as_u32(), mask)
}

pub fn read_allowed_mask_raw(tid: u32) -> io::Result<CpuMask> {
    // unchanged libc boundary
}

pub fn set_affinity_raw(tid: u32, mask: &CpuMask) -> io::Result<()> {
    // unchanged libc boundary
}
```

Update restore validation:

```rust
fn restore_record_status_at(
    proc_root: &Path,
    record: &AffinityRecord,
) -> io::Result<RestoreRecordStatus> {
    restore_identity_status_at(
        proc_root,
        record.tid,
        record.process_pid,
        record.process_starttime_ticks,
        record.task_starttime_ticks,
    )
}

pub(crate) fn restore_identity_status_at(
    proc_root: &Path,
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    if process_pid.is_none() && process_starttime_ticks.is_none() && task_starttime_ticks.is_none() {
        return Ok(RestoreRecordStatus::LegacyUnverified);
    }

    let (Some(process_pid), Some(process_starttime_ticks), Some(task_starttime_ticks)) =
        (process_pid, process_starttime_ticks, task_starttime_ticks)
    else {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    };

    let process_stat_path = proc_root.join(process_pid.to_string()).join("stat");

    // unchanged logic below
}
```

Update restore application:

```rust
match set_affinity(record.tid, &record.original_mask) {
    Ok(()) => summary.restored += 1,
    Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
        summary.skipped_dead += 1;
    }
    Err(err) => {
        summary.errors += 1;
        errors.push(affinity_set_error(record.tid, err));
    }
}
```

Update error helper:

```rust
fn affinity_set_error(tid: Tid, err: io::Error) -> anyhow::Error {
    anyhow::anyhow!("failed to set CPU affinity for TID {tid}: {err}")
}
```

Update merge key:

```rust
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RestoreMergeKey {
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}
```

Add compatibility test:

```rust
#[test]
fn affinity_record_typed_ids_preserve_numeric_json_shape() {
    let record = AffinityRecord {
        tid: Tid::new(7),
        process_pid: Some(Pid::new(42)),
        process_starttime_ticks: Some(100),
        task_starttime_ticks: Some(200),
        original_mask: CpuMask::parse("0-3").unwrap(),
        applied_mask: CpuMask::parse("0-1").unwrap(),
    };

    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["tid"], 7);
    assert_eq!(json["process_pid"], 42);

    let decoded: AffinityRecord = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.tid, Tid::new(7));
    assert_eq!(decoded.process_pid, Some(Pid::new(42)));
}
```

---

# Issue 3: `profile_restore.rs` persisted nice/ionice restore records still use raw IDs

## Current problematic code

`stutter/src/profile_restore.rs:25-53`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NiceRestoreRecordV2 {
    pub tid: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    // ...
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoPrioRestoreRecordV2 {
    pub tid: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    // ...
}
```

The restore functions also stay raw at `stutter/src/profile_restore.rs:273-318`, and the merge key is raw at `stutter/src/profile_restore.rs:347-353`.

## Fix plan

1. Import `Pid` / `Tid`.
2. Change `NiceRestoreRecordV2` and `IoPrioRestoreRecordV2` to typed IDs.
3. Change restore helper arguments to typed IDs.
4. Keep kernel/syscall-facing nice/ionice calls raw by converting at the final call site.
5. Type `RestoreIdentity.process_pid`.
6. Type `RestoreMergeKey`.
7. Add JSON compatibility tests for both records.

## Proposed code

```rust
use stutter_core::ids::{Pid, Tid};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NiceRestoreRecordV2 {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_nice: i32,
    pub applied_nice: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoPrioRestoreRecordV2 {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_ioprio: i32,
    pub applied_ioprio: i32,
}
```

Update operation closures in `restore_all_at`:

```rust
restore_all_at_with_ops(
    proc_root,
    state,
    affinity::set_affinity,
    |tid, nice| crate::actions::nice::set_task_nice(tid.as_u32(), nice).map_err(anyhow_to_io_error),
    |tid, ioprio| {
        crate::actions::ioprio::set_task_ioprio(tid.as_u32(), ioprio)
            .map_err(anyhow_to_io_error)
    },
)
```

Update generic bounds:

```rust
where
    FA: FnMut(Tid, &crate::affinity::CpuMask) -> io::Result<()>,
    FN: FnMut(Tid, i32) -> io::Result<()>,
    FI: FnMut(Tid, i32) -> io::Result<()>,
```

Update helper types:

```rust
fn restore_priority_identity(
    proc_root: &Path,
    tid: Tid,
    identity: RestoreIdentity,
    summary: &mut ProfileRestoreSummary,
    errors: &mut Vec<anyhow::Error>,
) -> bool {
    // unchanged logic, now typed
}

fn restore_record_status(
    proc_root: &Path,
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    affinity::restore_identity_status_at(
        proc_root,
        tid,
        process_pid,
        process_starttime_ticks,
        task_starttime_ticks,
    )
}

#[derive(Clone, Copy)]
struct RestoreIdentity {
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RestoreMergeKey {
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}
```

Update constructors in `profiles.rs` later to pass typed IDs directly:

```rust
plan.nice_records.push(NiceRestoreRecordV2 {
    tid: task.task_id(),
    process_pid: Some(task.process_id()),
    process_starttime_ticks: task.process_starttime_ticks,
    task_starttime_ticks: task.task_starttime_ticks,
    comm: Some(task.comm.clone()),
    original_nice,
    applied_nice: desired_nice,
});
```

Compatibility test:

```rust
#[test]
fn profile_restore_v2_typed_ids_preserve_numeric_json_shape() {
    let record = NiceRestoreRecordV2 {
        tid: Tid::new(11),
        process_pid: Some(Pid::new(10)),
        process_starttime_ticks: Some(100),
        task_starttime_ticks: Some(111),
        comm: Some("task".to_owned()),
        original_nice: 5,
        applied_nice: 10,
    };

    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["tid"], 11);
    assert_eq!(json["process_pid"], 10);

    let decoded: NiceRestoreRecordV2 = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.tid, Tid::new(11));
    assert_eq!(decoded.process_pid, Some(Pid::new(10)));
}
```

---

# Issue 4: `recorder/event_types.rs` is a serialized artifact schema, not the eBPF ABI

## Current problematic code

`stutter/src/recorder/event_types.rs` contains many serialized raw task/process IDs:

```rust
// lines 8-20
pub struct FocusEvent {
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
}

// lines 22-33
pub struct TreeEvent {
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
}

// lines 35-87
pub struct SpikeEvent {
    pub task: u32,
    pub process_pid: Option<u32>,
    pub switch_prev_pid: u32,
    pub waker_tid: u32,
}

// lines 89-96
pub struct MigrationEventRecord {
    pub tid: u32,
}

// lines 198-210
pub struct BlockIoRecord {
    pub tid: u32,
}

// lines 230-254
pub struct DrmFenceEventRecord {
    pub pid: Option<u32>,
    pub tid: Option<u32>,
}

// lines 340-360
pub struct GpuEngineSample {
    pub client_pid: Option<u32>,
}
```

The earlier claim that this file is the eBPF ring-buffer ABI is wrong. The actual ABI structs are in `stutter-common/src/lib.rs`, for example `SchedulerEvent` at lines 133-171, `MigrationEvent` at lines 217-225, `BlockIoEvent` at lines 264-275, `ExecEvent` at lines 284-291, `KmsFlipEvent` at lines 300-318, and `DrmFenceEvent` at lines 326-345. Those are `#[repr(C)]` and should **not** be changed to typed Rust wrappers.

## Fix plan

1. Leave `stutter-common/src/lib.rs` raw. That is the ABI.
2. Add a comment to `recorder/event_types.rs` making clear this file is the JSON/NDJSON artifact schema.
3. Import `Pid` / `Tid`.
4. Change recorder artifact fields to typed IDs while preserving JSON numeric shape through `#[serde(transparent)]`.
5. Update constructors at eBPF conversion boundaries with `Tid::new(raw)` / `Pid::new(raw)`.
6. Update report/diagnosis code that uses these fields as map keys or numeric values to call `.as_u32()` where needed.
7. Add JSON compatibility tests for legacy numeric NDJSON.

## Proposed code

At the top of `stutter/src/recorder/event_types.rs`:

```rust
use stutter_core::ids::{Pid, Tid};
```

Add file-level comment:

```rust
// These are recorder artifact DTOs, not the eBPF ring-buffer ABI.
// Keep field names and JSON numeric shape stable. Rust-side task/process IDs
// are typed with serde-transparent wrappers; convert to raw u32 only when
// crossing eBPF, procfs, libc, or JavaScript/report-template boundaries.
```

Update DTOs:

```rust
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct FocusEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub old_kind: Option<String>,
    pub kind: Option<String>,
    pub root_pids: Vec<Pid>,
    pub member_pids: Vec<Pid>,
    pub confidence: f32,
    pub score: f32,
    pub situation: Option<crate::autotune::state::SituationKind>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct TreeEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub tid: Tid,
    pub process_pid: Pid,
    pub process_ppid: Pid,
    pub comm: String,
    pub process_comm: String,
    pub class: TaskClass,
    pub from_cgroup: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SpikeEvent {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    pub task: Tid,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<Pid>,
    pub process_comm: String,
    pub comm: String,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: Tid,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: Tid,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}
```

Update the conversion from `SchedulerEvent`:

```rust
pub struct SpikeDiagnosticContext {
    pub scx_ops: Option<String>,
    pub scx_state: Option<String>,
    pub scx_enable_seq: Option<String>,
    pub cause_tags: Vec<String>,
    pub primary_cause: Option<String>,
    pub waker_tid: Tid,
    pub waker_comm: String,
}

impl SpikeEvent {
    pub fn from_task_stats(
        monotonic_start_ns: Option<u64>,
        stats: &TaskStats,
        event: &SchedulerEvent,
        fault_deltas: (u64, u64),
        diag: SpikeDiagnosticContext,
    ) -> Self {
        Self {
            elapsed_ms: crate::recorder::session::elapsed_ms_from_monotonic(
                monotonic_start_ns,
                event.switch_ns,
            ),
            task: Tid::new(event.tid),
            active: stats.active,
            class: stats.class,
            process_pid: stats.process_id(),
            process_comm: stats.process_comm.clone(),
            comm: stats.comm.clone(),
            cpu: event.cpu,
            wakeup_target_cpu: event.wakeup_target_cpu,
            prio: event.prio,
            latency_ns: event.latency_ns,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            switch_prev_pid: Tid::new(event.switch_prev_pid),
            switch_prev_state: event.switch_prev_state,
            switch_prev_state_label: crate::sched_state::classify_switch_prev_state(
                event.switch_prev_state,
            )
            .to_owned(),
            waker_tid: diag.waker_tid,
            waker_comm: diag.waker_comm,
            target_pending_wakeups: event.target_pending_wakeups,
            observed_runnable_depth: event.observed_runnable_depth,
            major_faults: fault_deltas.0,
            minor_faults: fault_deltas.1,
            scx_ops: diag.scx_ops,
            scx_state: diag.scx_state,
            scx_enable_seq: diag.scx_enable_seq,
            cause_tags: diag.cause_tags,
            primary_cause: diag.primary_cause,
        }
    }
}
```

Other recorder DTO changes:

```rust
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct MigrationEventRecord {
    pub elapsed_ms: u64,
    pub tid: Tid,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct BlockIoRecord {
    pub elapsed_ms: u64,
    pub tid: Tid,
    #[serde(default = "default_block_io_correlation_basis")]
    pub correlation_basis: Cow<'static, str>,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct DrmFenceEventRecord {
    pub elapsed_ms: u64,
    pub timestamp_ns: u64,
    pub source: String,
    pub event_kind: String,
    pub driver: Option<String>,
    pub card: Option<String>,
    pub gpu_role: Option<String>,
    pub pid: Option<Pid>,
    pub tid: Option<Tid>,
    pub comm: Option<String>,
    pub context: Option<u64>,
    pub seqno: Option<u64>,
    pub timeline_hash: Option<u64>,
    pub wait_start_ns: Option<u64>,
    pub wait_done_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_ns: Option<u64>,
    pub duration_ns: Option<u64>,
    pub exporter_driver: Option<String>,
    pub importer_driver: Option<String>,
    pub correlation_basis: String,
    pub confidence: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct GpuEngineSample {
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drm_card: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_pid: Option<Pid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_comm: Option<String>,
    pub source: String,
    pub confidence: String,
}
```

Compatibility test:

```rust
#[test]
fn spike_event_typed_ids_preserve_legacy_numeric_json() {
    let json = r#"{
        "elapsed_ms": 1,
        "task": 1234,
        "active": true,
        "class": "Game",
        "process_pid": 1000,
        "process_comm": "Game.exe",
        "comm": "render",
        "cpu": 2,
        "prio": 120,
        "latency_ns": 5000,
        "wakeup_ns": 10,
        "switch_ns": 20,
        "target_pending_wakeups": 1
    }"#;

    let event: SpikeEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.task, Tid::new(1234));
    assert_eq!(event.process_pid, Some(Pid::new(1000)));

    let encoded = serde_json::to_value(&event).unwrap();
    assert_eq!(encoded["task"], 1234);
    assert_eq!(encoded["process_pid"], 1000);
}
```

---

# Issue 5: `autotune/washout.rs` test-only identities still use raw IDs

## Current problematic code

`stutter/src/autotune/washout.rs:61-68`:

```rust
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WashoutTargetSnapshot {
    pub target_present: bool,
    pub root_pid: u32,
    pub active_target_count: usize,
    pub identities: BTreeSet<WashoutTaskIdentity>,
}
```

`stutter/src/autotune/washout.rs:115-127`:

```rust
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WashoutTaskIdentity {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub process_comm: String,
    // ...
}
```

`from_task_info` converts typed IDs back to raw at `stutter/src/autotune/washout.rs:131-135`.

This is low-risk because it is `#[cfg(test)]`, but it is still real typed-ID drift.

## Fix plan

1. Import `Pid` / `Tid` under `#[cfg(test)]`.
2. Change `WashoutTargetSnapshot.root_pid` to `Pid`.
3. Change `WashoutTaskIdentity.tid` to `Tid`.
4. Change `WashoutTaskIdentity.process_pid` to `Pid`.
5. Keep `target_snapshot` input raw by converting with `.as_u32()` at that boundary.
6. Update test helpers.

## Proposed code

```rust
#[cfg(test)]
use stutter_core::ids::{Pid, Tid};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WashoutTargetSnapshot {
    pub target_present: bool,
    pub root_pid: Pid,
    pub active_target_count: usize,
    pub identities: BTreeSet<WashoutTaskIdentity>,
}

#[cfg(test)]
impl WashoutTargetSnapshot {
    pub fn absent(root_pid: Pid) -> Self {
        Self {
            target_present: false,
            root_pid,
            active_target_count: 0,
            identities: BTreeSet::new(),
        }
    }

    pub fn from_target_snapshot(root_pid: Pid, snapshot: &TargetSnapshot) -> Self {
        let identities = snapshot
            .tasks
            .values()
            .map(WashoutTaskIdentity::from_task_info)
            .collect::<BTreeSet<_>>();

        Self {
            target_present: snapshot.process_roots.contains(&root_pid.as_u32()) && !identities.is_empty(),
            root_pid,
            active_target_count: identities.len(),
            identities,
        }
    }

    pub fn capture(root_pid: Pid) -> Self {
        Self::capture_at(Path::new("/proc"), root_pid)
    }

    pub fn capture_at(proc_root: &Path, root_pid: Pid) -> Self {
        if root_pid.as_u32() == 0 {
            return Self::absent(root_pid);
        }

        let tree_pids = [root_pid.as_u32()];
        let snapshot = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc_root)
                .tree_pids(&tree_pids),
        );
        Self::from_target_snapshot(root_pid, &snapshot)
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WashoutTaskIdentity {
    pub tid: Tid,
    pub process_pid: Pid,
    pub comm: String,
    pub process_comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub class: TaskClass,
}

#[cfg(test)]
impl WashoutTaskIdentity {
    pub fn from_task_info(task: &TaskInfo) -> Self {
        Self {
            tid: task.task_id(),
            process_pid: task.process_id(),
            comm: task.comm.clone(),
            process_comm: task.process_comm.clone(),
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            exe_dev: task.exe_dev,
            exe_ino: task.exe_ino,
            class: task.class,
        }
    }
}
```

Also update `run_washout_for_action` at `stutter/src/autotune/washout.rs:286-315`:

```rust
pub async fn run_washout_for_action<A: TuningAction>(
    action: &A,
    tree_pid: Pid,
    config: WashoutWindowConfig,
) -> anyhow::Result<()> {
    let started_unix_nanos = unix_nanos_now();
    let initial_target = WashoutTargetSnapshot::capture(tree_pid);

    // unchanged below
}
```

---

# Issue 6: `profiles.rs::ProfileTaskPlan` and `ProfileApplyCacheKey` still use raw IDs

## Current problematic code

Private cache key at `stutter/src/profiles.rs:68-76`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProfileApplyCacheKey {
    tid: u32,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    desired_affinity: Option<CpuMask>,
    desired_nice: Option<i32>,
    desired_ionice: Option<IoPrioValue>,
}
```

Public-ish plan type at `stutter/src/profiles.rs:89-98`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTaskPlan {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}
```

Constructor converts typed `ActiveTaskSnapshot` fields back to raw at `stutter/src/profiles.rs:111-119`:

```rust
Some(ProfileTaskPlan {
    tid: task.tid.as_u32(),
    process_pid: task.process_pid.as_u32(),
    comm: task.comm.clone(),
    class: task.class,
    requested_mask,
    matched_rule_index: rule_index,
    matched_rule_name: None,
})
```

This matters because `ProfileTaskPlan` is used by active-config matching/rollback, not just tests. Raw lookup usage exists in `stutter/src/autotune/active_config/rollback.rs:350-370` and `stutter/src/autotune/active_config/matching.rs:444-456`.

## Fix plan

1. Import `Pid` / `Tid` in `profiles.rs`.
2. Change `ProfileApplyCacheKey.tid` to `Tid`.
3. Change `ProfileTaskPlan.tid` to `Tid`.
4. Change `ProfileTaskPlan.process_pid` to `Pid`.
5. Stop calling `.as_u32()` when constructing the plan.
6. Update active-config lookup code to call `.as_u32()` only when indexing current raw snapshot maps.
7. Update result sorting and tests.

## Proposed code

At the top of `profiles.rs`:

```rust
use stutter_core::ids::{Pid, Tid};
```

Update cache key:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProfileApplyCacheKey {
    tid: Tid,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    desired_affinity: Option<CpuMask>,
    desired_nice: Option<i32>,
    desired_ionice: Option<IoPrioValue>,
}

impl ProfileApplyCacheKey {
    fn new(task: &TaskInfo, rule: &ProfileRule) -> Self {
        Self {
            tid: task.task_id(),
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            desired_affinity: rule.affinity.clone(),
            desired_nice: rule.nice,
            desired_ionice: rule.ionice,
        }
    }
}
```

Update `ProfileTaskPlan`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTaskPlan {
    pub tid: Tid,
    pub process_pid: Pid,
    pub comm: String,
    pub class: TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}
```

Update construction:

```rust
Some(ProfileTaskPlan {
    tid: task.tid,
    process_pid: task.process_pid,
    comm: task.comm.clone(),
    class: task.class,
    requested_mask,
    matched_rule_index: rule_index,
    matched_rule_name: None,
})
```

Update pending sets in profile application:

```rust
let mut pending_tids = BTreeSet::<Tid>::new();

pending_tids.insert(task.task_id());
```

Update `AffinityRecord` construction after the record is typed:

```rust
plan.affinity_changes.push(PlannedAffinityChange {
    record: AffinityRecord {
        tid: task.task_id(),
        process_pid: Some(task.process_id()),
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        original_mask,
        applied_mask: desired_mask.clone(),
    },
});
```

Update active-config lookups.

Current `rollback.rs:350-357`:

```rust
let Some(expected) = baseline.affinity.per_tid.get(&task.tid) else {
```

Proposed:

```rust
let tid = task.tid.as_u32();

let Some(expected) = baseline.affinity.per_tid.get(&tid) else {
    return RollbackVerification::unavailable(format!(
        "tid={} baseline CPU affinity missing",
        task.tid
    ));
};

let Some(actual) = post_rollback.affinity.per_tid.get(&tid) else {
    return RollbackVerification::mismatch(
        format!("tid={} cpu_affinity={expected}", task.tid),
        format!("tid={} cpu_affinity=missing", task.tid),
        "rollback_target_missing",
    );
};
```

Current `matching.rs:444-456`:

```rust
for task in &planned_tasks {
    let Some(current) = snapshot.affinity.per_tid.get(&task.tid) else {
        return ActiveConfigMatch::Unknown {
            summary: format!("tid={} active CPU affinity missing", task.tid),
        };
    };
```

Proposed:

```rust
for task in &planned_tasks {
    let tid = task.tid.as_u32();

    let Some(current) = snapshot.affinity.per_tid.get(&tid) else {
        return ActiveConfigMatch::Unknown {
            summary: format!("tid={} active CPU affinity missing", task.tid),
        };
    };

    if !cpu_mask_strings_match(current, &task.requested_mask) {
        return ActiveConfigMatch::Differs {
            expected: format!("tid={} cpu_affinity={}", task.tid, task.requested_mask),
            actual: format!("tid={} cpu_affinity={current}", task.tid),
        };
    }
}
```

---

# Architecture tests to prevent regression

The existing architecture test already protects `actions/model.rs` from returning to raw `pub tid: u32` / `pub process_pid: Option<u32>` at `stutter/src/architecture_tests/typed_ids.rs:55-83`.

Add a new test covering the files from this report.

## Proposed code

```rust
#[test]
fn remaining_task_process_identity_models_use_typed_ids() {
    for relative_path in [
        "focus/classify.rs",
        "affinity.rs",
        "profile_restore.rs",
        "autotune/washout.rs",
        "profiles.rs",
    ] {
        let source = source(relative_path);
        assert!(
            !source.contains("pub tid: u32")
                && !source.contains("pub process_pid: u32")
                && !source.contains("pub process_pid: Option<u32>")
                && !source.contains("pub pid: u32")
                && !source.contains("pub ppid: u32"),
            "{relative_path} must use Tid/Pid for public or persisted task/process identity fields"
        );
    }
}

#[test]
fn recorder_artifact_task_process_ids_use_typed_ids_but_common_abi_stays_raw() {
    let recorder = source("recorder/event_types.rs");
    assert!(
        recorder.contains("use stutter_core::ids::{Pid, Tid};")
            && recorder.contains("pub tid: Tid")
            && recorder.contains("pub process_pid: Pid")
            && recorder.contains("pub process_pid: Option<Pid>")
            && recorder.contains("pub task: Tid"),
        "recorder artifact DTOs should keep Rust-side task/process IDs typed while serde preserves numeric JSON"
    );

    // The eBPF ABI lives outside stutter/src, so do not assert against it here.
}
```

A stronger final grep after the migration should be:

```bash
rg -n 'pub (tid|process_pid|pid|ppid|task|waker_tid|switch_prev_pid|client_pid): (Option<)?u32' \
  stutter/src/focus/classify.rs \
  stutter/src/affinity.rs \
  stutter/src/profile_restore.rs \
  stutter/src/recorder/event_types.rs \
  stutter/src/autotune/washout.rs \
  stutter/src/profiles.rs
```

Expected result after the full migration: no matches, except deliberately raw syscall/helper/test function parameters.

---

# Deliberately not fixed

These are real raw `u32`s, but they should remain raw or be handled separately:

1. **`stutter-common/src/lib.rs` eBPF ABI structs**
   These are `#[repr(C)]` ring-buffer records and must stay fixed-width raw integers. Do not replace fields like `SchedulerEvent.tid`, `MigrationEvent.tid`, `BlockIoEvent.tid`, `ExecEvent.pid`, `ExecEvent.tid`, `KmsFlipEvent.pid`, or `DrmFenceEvent.tid` with Rust wrappers.

2. **Low-level syscall/procfs functions**
   Functions like `read_allowed_mask_raw(tid: u32)` and `set_affinity_raw(tid: u32, ...)` in `affinity.rs:225-257` are explicitly raw kernel/libc boundaries. The fix is to add typed wrappers and use those wrappers in higher-level code, not to remove every raw parameter from the lowest layer.

3. **Non-task numeric fields**
   Fields like CPU IDs, IRQ numbers, CRTC IDs, sectors, GPU card minors, and frequencies are not part of this task/process ID migration. They may later deserve `CpuId` / `IrqId`, but that is a separate migration.

---

# Recommended implementation order

1. Add `Default` to the numeric ID macro in `stutter-core/src/ids.rs`.
2. Fix `focus/classify.rs` first. It is not serialized and will expose simple call-site errors.
3. Fix `affinity.rs` restore records and add typed affinity wrappers.
4. Fix `profile_restore.rs` and update `profiles.rs` construction of restore records.
5. Fix `profiles.rs::ProfileTaskPlan` and active-config lookup conversions.
6. Fix `autotune/washout.rs` because it is test-only and low risk.
7. Fix `recorder/event_types.rs` last because it has the widest call-site blast radius.
8. Add serde compatibility tests for affinity, profile restore, and recorder events.
9. Add the architecture tests.
10. Run:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
```

Bottom line: **the real fix is not “make every `u32` disappear.” The right fix is: type every internal/persisted task/process identity, preserve numeric JSON with `serde(transparent)`, and keep raw `u32` only at eBPF, libc, procfs, and report-template/serialization edges where raw numbers are the actual contract.**
