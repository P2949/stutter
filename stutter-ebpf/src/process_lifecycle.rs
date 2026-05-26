use aya_ebpf::{
    helpers::bpf_get_current_pid_tgid,
    programs::{PerfEventContext, TracePointContext},
};
use stutter_common::{EVENT_EXEC, ExecEvent};

use crate::{
    maps::{FaultCounters, PREV_FAULTS},
    runnable_depth::{decrement_target_pending, remove_runnable_task_if_present},
    target_filter::{is_target_pid, is_target_pid_or_current_cgroup},
    wakeup_data::{self, WakeupData},
};

#[inline(always)]
pub(crate) fn try_sched_process_exec(_ctx: TracePointContext) -> u32 {
    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xffff_ffff) as u32;

    if !is_target_pid(pid) && !is_target_pid_or_current_cgroup(tid) {
        return 0;
    }

    let comm = aya_ebpf::helpers::bpf_get_current_comm().unwrap_or([0; 16]);
    emit_ringbuf_event!(
        ExecEvent,
        return 0,
        ExecEvent {
            kind: EVENT_EXEC,
            pid,
            tid,
            comm,
        }
    );

    0
}

#[inline(always)]
pub(crate) fn try_sched_process_exit(_ctx: TracePointContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;

    remove_runnable_task_if_present(tid);

    let mut old = WakeupData::default();
    if wakeup_data::remove_for_exit(tid, &mut old) {
        decrement_target_pending(old.target_cpu);
    }
    let _ = PREV_FAULTS.remove(tid);
    0
}

#[inline(always)]
pub(crate) fn try_major_fault(_ctx: PerfEventContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    if is_target_pid_or_current_cgroup(tid) {
        if let Some(counters) = PREV_FAULTS.get_ptr_mut(tid) {
            unsafe { (*counters).maj += 1 };
        } else {
            let _ = PREV_FAULTS.insert(tid, FaultCounters { maj: 1, min: 0 }, 0);
        }
    }
    0
}

#[inline(always)]
pub(crate) fn try_minor_fault(_ctx: PerfEventContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    if is_target_pid_or_current_cgroup(tid) {
        if let Some(counters) = PREV_FAULTS.get_ptr_mut(tid) {
            unsafe { (*counters).min += 1 };
        } else {
            let _ = PREV_FAULTS.insert(tid, FaultCounters { maj: 0, min: 1 }, 0);
        }
    }
    0
}
