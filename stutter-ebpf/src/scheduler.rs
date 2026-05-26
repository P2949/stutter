use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_smp_processor_id, bpf_ktime_get_ns},
    programs::TracePointContext,
};
use stutter_common::{
    DROP_WAKEUP_DATA_CONSUMED_READ_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED,
    DROP_WAKEUP_DATA_REPLACED_ENTRY, DROP_WAKEUP_DATA_STALE_ENTRY, EVENT_MIGRATION,
    EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, MigrationEvent, SchedulerEvent, StatWaitEvent,
    tracepoint_offsets::{
        SCHED_MIGRATE_TASK_DEST_CPU_OFFSET, SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET,
        SCHED_MIGRATE_TASK_PID_OFFSET, SCHED_STAT_WAIT_DELAY_OFFSET, SCHED_STAT_WAIT_PID_OFFSET,
        SCHED_SWITCH_NEXT_COMM_OFFSET, SCHED_SWITCH_NEXT_PID_OFFSET, SCHED_SWITCH_NEXT_PRIO_OFFSET,
        SCHED_SWITCH_PREV_PID_OFFSET, SCHED_SWITCH_PREV_STATE_OFFSET, SCHED_WAKEUP_PID_OFFSET,
        SCHED_WAKEUP_TARGET_CPU_OFFSET,
    },
};

use crate::{
    drop_counters::increment_drop_counter,
    maps::{FaultCounters, PREV_FAULTS, RUNNABLE_TASK_CPU, TARGET_PENDING_WAKEUPS},
    runnable_depth::{
        decrement_target_pending, increment_target_pending, mark_task_runnable, mark_task_running,
        read_cpu_runnable_depth, remove_runnable_task_if_present,
    },
    target_filter::{is_target_pid, valid_cpu},
    trace_read::{read_comm16, read_i32, read_i64, read_u32, read_u64},
    wakeup_data::{
        self, WAKEUP_CONSUME_CURSOR_INSERT_FAILED, WAKEUP_CONSUME_NONE, WAKEUP_CONSUME_OK,
        WAKEUP_RECORD_INSERT_FAILED, WAKEUP_RECORD_NEW_PENDING,
        WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU, WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU,
        WakeupData,
    },
};

#[inline(always)]
pub(crate) fn try_sched_wakeup(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_WAKEUP_PID_OFFSET, &mut pid) {
        return 1;
    }
    let mut target_cpu: u32 = 0;
    if !read_u32(&ctx, SCHED_WAKEUP_TARGET_CPU_OFFSET, &mut target_cpu) {
        return 1;
    }

    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;

    // Keep PID filtering here. current_cgroup is the waker, not the wakee.
    // Runnable-depth accounting is intentionally target-local: only monitored
    // target tasks are counted. Do not call mark_task_runnable() before this
    // filter, or unrelated system wakeups can leak into CPU_RUNNABLE_DEPTH.
    if !is_target_pid(pid) {
        return 0;
    }

    let now = unsafe { bpf_ktime_get_ns() };
    let waker_tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;

    let mut seq = 0;
    if !wakeup_data::next_wakeup_seq(pid, &mut seq) {
        increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED);
        return 0;
    }

    let data = WakeupData {
        ts: now,
        target_cpu,
        waker_tid,
        seq,
    };
    let mut old = WakeupData::default();
    match wakeup_data::record_wakeup(pid, data, &mut old) {
        WAKEUP_RECORD_INSERT_FAILED => {
            increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED);
            return 0;
        }
        WAKEUP_RECORD_NEW_PENDING => {
            // Count only monitored target tasks as runnable after WAKEUP_DATA
            // accepts the wakeup. If the wakeup record cannot be installed,
            // there is no later sched_switch consume path to clean up
            // runnable-depth accounting.
            let target_cpu_tracked = mark_task_runnable(pid, target_cpu);
            if target_cpu_tracked {
                increment_target_pending(target_cpu);
            }
        }
        WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU => {
            increment_drop_counter(DROP_WAKEUP_DATA_REPLACED_ENTRY);
            decrement_target_pending(old.target_cpu);
            let target_cpu_tracked = mark_task_runnable(pid, target_cpu);
            if target_cpu_tracked {
                increment_target_pending(target_cpu);
            }
        }
        WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU => {
            let _ = mark_task_runnable(pid, target_cpu);
            increment_drop_counter(DROP_WAKEUP_DATA_REPLACED_ENTRY);
        }
        _ => {}
    }

    0
}

#[inline(always)]
pub(crate) fn try_sched_switch(ctx: TracePointContext) -> u32 {
    let mut next_pid: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_NEXT_PID_OFFSET, &mut next_pid) {
        return 1;
    }

    if next_pid <= 0 {
        return 0;
    }

    let pid = next_pid as u32;

    let mut wakeup_data = WakeupData::default();
    match wakeup_data::consume_pending_wakeup(pid, &mut wakeup_data) {
        WAKEUP_CONSUME_OK => {}
        WAKEUP_CONSUME_CURSOR_INSERT_FAILED => {
            increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED);
            if wakeup_data::drop_pending_wakeup_after_cursor_failure(pid, &wakeup_data) {
                decrement_target_pending(wakeup_data.target_cpu);
                remove_runnable_task_if_present(pid);
            }
            return 0;
        }
        WAKEUP_CONSUME_NONE => return 0,
        _ => return 0,
    }

    if !is_target_pid(pid) {
        increment_drop_counter(DROP_WAKEUP_DATA_STALE_ENTRY);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 0;
    }

    // Wakeup data is marked consumed before slower tracepoint reads to avoid stale state.

    // Read the previous task context only after the cheap relevance filters pass.
    // Offsets are named above and validated in userspace preflight before load.
    let mut prev_pid_raw: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_PREV_PID_OFFSET, &mut prev_pid_raw) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 1;
    }
    let mut prev_state: i64 = 0;
    if !read_i64(&ctx, SCHED_SWITCH_PREV_STATE_OFFSET, &mut prev_state) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 1;
    }
    let switch_prev_pid = if prev_pid_raw > 0 {
        prev_pid_raw as u32
    } else {
        0
    };

    let wakeup_ns = wakeup_data.ts;
    let waker_tid = wakeup_data.waker_tid;
    let target_cpu = wakeup_data.target_cpu;

    // We only arrived here because a wakeup record for this PID exists
    // (inserted by sched_wakeup). Treat that as sufficient evidence this is
    // a target-related event.

    let cpu = unsafe { bpf_get_smp_processor_id() } as u32;

    // Every sched_switch means next_pid is now running, so it is no longer
    // counted as runnable in our approximation.
    mark_task_running(pid, cpu);

    // Read monitored runnable depth after removing next_pid. This represents
    // remaining monitored target tasks that are still counted runnable on this CPU,
    // not total kernel runqueue depth.
    let observed_runnable_depth = read_cpu_runnable_depth(cpu);

    // Decrement the target-pending counter for the CPU where the task was
    // originally queued. This is not kernel runqueue depth.
    decrement_target_pending(target_cpu);

    let target_pending_wakeups = if valid_cpu(cpu) {
        TARGET_PENDING_WAKEUPS.get(cpu).copied().unwrap_or(0)
    } else {
        0
    };

    let switch_ns = unsafe { bpf_ktime_get_ns() };
    let latency_ns = switch_ns.saturating_sub(wakeup_ns);

    let faults = unsafe {
        PREV_FAULTS
            .get(pid)
            .copied()
            .unwrap_or(FaultCounters { maj: 0, min: 0 })
    };

    let mut prio: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_NEXT_PRIO_OFFSET, &mut prio) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        return 1;
    }
    let mut comm: [u8; 16] = [0; 16];
    if !read_comm16(&ctx, SCHED_SWITCH_NEXT_COMM_OFFSET, &mut comm) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        return 1;
    }

    emit_ringbuf_event!(
        SchedulerEvent,
        return 0,
        SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: pid,
            cpu,
            wakeup_target_cpu: target_cpu,
            prio,
            waker_tid,
            target_pending_wakeups,
            observed_runnable_depth,
            maj_flt: faults.maj,
            min_flt: faults.min,
            wakeup_ns,
            switch_ns,
            latency_ns,
            comm,
            switch_prev_pid,
            _pad0: 0,
            switch_prev_state: prev_state,
        }
    );

    0
}

#[inline(always)]
pub(crate) fn try_sched_migrate_task(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_PID_OFFSET, &mut pid) {
        return 1;
    }
    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return 0;
    }

    let mut orig_cpu: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET, &mut orig_cpu) {
        return 1;
    }
    let mut dest_cpu: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_DEST_CPU_OFFSET, &mut dest_cpu) {
        return 1;
    }
    let now = unsafe { bpf_ktime_get_ns() };

    // Move monitored runnable count if this target task migrates while runnable.
    let new_cpu = dest_cpu as u32;
    match unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        Some(old_cpu) if old_cpu != new_cpu => {
            mark_task_runnable(pid, new_cpu);
        }
        _ => {}
    }

    // If this task is currently a monitored target with a pending wakeup,
    // update its target CPU and move its diagnostic-only pending counter.
    let mut old_cpu = 0;
    if wakeup_data::move_pending_cpu(pid, new_cpu, &mut old_cpu) {
        decrement_target_pending(old_cpu);
        increment_target_pending(new_cpu);
    }

    emit_ringbuf_event!(
        MigrationEvent,
        return 0,
        MigrationEvent {
            kind: EVENT_MIGRATION,
            tid: pid,
            from_cpu: orig_cpu as u32,
            to_cpu: dest_cpu as u32,
            timestamp_ns: now,
        }
    );

    0
}

#[inline(always)]
pub(crate) fn try_sched_stat_wait(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_STAT_WAIT_PID_OFFSET, &mut pid) {
        return 1;
    }
    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return 0;
    }

    let mut delay: u64 = 0;
    if !read_u64(&ctx, SCHED_STAT_WAIT_DELAY_OFFSET, &mut delay) {
        return 1;
    }

    emit_ringbuf_event!(
        StatWaitEvent,
        return 0,
        StatWaitEvent {
            kind: EVENT_STAT_WAIT,
            tid: pid,
            delay_ns: delay,
        }
    );

    0
}
