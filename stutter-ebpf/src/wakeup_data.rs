//! Wakeup handoff state shared by sched_wakeup and sched_switch probes.
//!
//! `WAKEUP_DATA` stores the latest wakeup observed for a task. `WAKEUP_CONSUMED` is
//! not a second queue; it is a cursor that records the exact `WakeupData` value a
//! sched_switch probe already consumed. A pending wakeup is therefore defined as
//! "present in `WAKEUP_DATA` and not exactly equal to the consumed cursor".
//!
//! Keeping the data map entry after consumption avoids the copy-then-delete race
//! where sched_switch could delete a newer wakeup inserted between lookup and
//! removal. Writers clear the consumed cursor when installing a new wakeup, task
//! exit removes both maps, and CPU migration only mutates wakeups that are still
//! pending under the cursor rule above.
//!
//! To prevent ABA collisions where a new wakeup happens to have the same exact
//! timestamp, CPU, and waker TID as a previously consumed wakeup, `WakeupData`
//! includes a per-TID monotonically increasing sequence number `seq`.

use aya_ebpf::{macros::map, maps::HashMap};

use crate::map_limits::WAKEUP_DATA_MAP_MAX_ENTRIES;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct WakeupData {
    pub(crate) ts: u64,
    pub(crate) target_cpu: u32,
    pub(crate) waker_tid: u32,
    pub(crate) seq: u32,
}

#[map]
static WAKEUP_DATA: HashMap<u32, WakeupData> =
    HashMap::<u32, WakeupData>::with_max_entries(WAKEUP_DATA_MAP_MAX_ENTRIES, 0);

#[map]
static WAKEUP_CONSUMED: HashMap<u32, WakeupData> =
    HashMap::<u32, WakeupData>::with_max_entries(WAKEUP_DATA_MAP_MAX_ENTRIES, 0);

#[map]
static WAKEUP_SEQ: HashMap<u32, u32> =
    HashMap::<u32, u32>::with_max_entries(WAKEUP_DATA_MAP_MAX_ENTRIES, 0);

#[inline(always)]
pub(crate) fn next_wakeup_seq(pid: u32) -> u32 {
    let next = match unsafe { WAKEUP_SEQ.get(pid) } {
        Some(seq) => seq.wrapping_add(1),
        None => 1,
    };

    let _ = WAKEUP_SEQ.insert(pid, next, 0);
    next
}

pub(crate) const WAKEUP_RECORD_INSERT_FAILED: u32 = 0;
pub(crate) const WAKEUP_RECORD_NEW_PENDING: u32 = 1;
pub(crate) const WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU: u32 = 2;
pub(crate) const WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU: u32 = 3;
pub(crate) const WAKEUP_CONSUME_NONE: u32 = 0;
pub(crate) const WAKEUP_CONSUME_OK: u32 = 1;
pub(crate) const WAKEUP_CONSUME_CURSOR_INSERT_FAILED: u32 = 2;

#[inline(always)]
pub(crate) fn record_wakeup(pid: u32, data: WakeupData, old: &mut WakeupData) -> u32 {
    let mut had_pending_old = false;

    if let Some(existing) = unsafe { WAKEUP_DATA.get(pid) } {
        *old = *existing;
        had_pending_old = !same_wakeup_consumed(pid, old);
    }

    if WAKEUP_DATA.insert(pid, data, 0).is_err() {
        return WAKEUP_RECORD_INSERT_FAILED;
    }

    // A new wakeup supersedes any cursor for an older consumed wakeup. Since
    // each new wakeup gets a unique sequence number via `next_wakeup_seq`, if a
    // concurrent sched_switch later records an older wakeup as consumed, the
    // cursor still will not match this newer map value.
    let _ = WAKEUP_CONSUMED.remove(pid);

    if !had_pending_old {
        return WAKEUP_RECORD_NEW_PENDING;
    }

    if old.target_cpu == data.target_cpu {
        WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU
    } else {
        WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU
    }
}

#[inline(always)]
pub(crate) fn consume_pending_wakeup(pid: u32, out: &mut WakeupData) -> u32 {
    let Some(data) = (unsafe { WAKEUP_DATA.get(pid) }) else {
        return WAKEUP_CONSUME_NONE;
    };

    *out = *data;
    if same_wakeup_consumed(pid, out) {
        return WAKEUP_CONSUME_NONE;
    }

    // Mark this exact wakeup as consumed without deleting WAKEUP_DATA. That
    // removes the copy-then-delete race where sched_switch could delete a newer
    // wakeup inserted after its lookup.
    if WAKEUP_CONSUMED.insert(pid, *out, 0).is_err() {
        return WAKEUP_CONSUME_CURSOR_INSERT_FAILED;
    }

    WAKEUP_CONSUME_OK
}

#[inline(always)]
pub(crate) fn remove_for_exit(pid: u32, out: &mut WakeupData) -> bool {
    let mut was_pending = false;

    if let Some(data) = unsafe { WAKEUP_DATA.get(pid) } {
        *out = *data;
        was_pending = !same_wakeup_consumed(pid, out);
        let _ = WAKEUP_DATA.remove(pid);
    }

    let _ = WAKEUP_CONSUMED.remove(pid);
    let _ = WAKEUP_SEQ.remove(pid);
    was_pending
}

#[inline(always)]
pub(crate) fn move_pending_cpu(pid: u32, new_cpu: u32, old_cpu: &mut u32) -> bool {
    let Some(data) = WAKEUP_DATA.get_ptr_mut(pid) else {
        return false;
    };

    let current = unsafe { *data };
    if same_wakeup_consumed(pid, &current) || current.target_cpu == new_cpu {
        return false;
    }

    *old_cpu = current.target_cpu;
    unsafe { (*data).target_cpu = new_cpu };
    true
}

#[inline(always)]
fn same_wakeup_consumed(pid: u32, data: &WakeupData) -> bool {
    match unsafe { WAKEUP_CONSUMED.get(pid) } {
        Some(consumed) => {
            consumed.ts == data.ts
                && consumed.target_cpu == data.target_cpu
                && consumed.waker_tid == data.waker_tid
                && consumed.seq == data.seq
        }
        None => false,
    }
}
