use stutter_common::DROP_CPU_ACCOUNTING_UNTRACKED;

use crate::{
    drop_counters::increment_drop_counter,
    maps::{CPU_RUNNABLE_DEPTH, RUNNABLE_TASK_CPU, TARGET_PENDING_WAKEUPS},
    target_filter::valid_cpu,
};

#[inline(always)]
pub(crate) fn read_cpu_runnable_depth(cpu: u32) -> u32 {
    if !valid_cpu(cpu) {
        return 0;
    }
    CPU_RUNNABLE_DEPTH.get(cpu).copied().unwrap_or(0)
}

#[inline(always)]
pub(crate) fn increment_cpu_runnable_depth(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = CPU_RUNNABLE_DEPTH.get_ptr_mut(cpu) {
        unsafe { *depth = (*depth).saturating_add(1) };
    }
}

#[inline(always)]
pub(crate) fn decrement_cpu_runnable_depth(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = CPU_RUNNABLE_DEPTH.get_ptr_mut(cpu) {
        unsafe { *depth = (*depth).saturating_sub(1) };
    }
}

#[inline(always)]
pub(crate) fn mark_task_runnable(pid: u32, target_cpu: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if !valid_cpu(target_cpu) {
        increment_drop_counter(DROP_CPU_ACCOUNTING_UNTRACKED);
        return false;
    }

    match unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        Some(old_cpu) if old_cpu == target_cpu => {
            // Already counted on the same CPU.
        }
        Some(old_cpu) => {
            // Migrated while runnable.
            decrement_cpu_runnable_depth(old_cpu);
            increment_cpu_runnable_depth(target_cpu);
            let _ = RUNNABLE_TASK_CPU.insert(pid, target_cpu, 0);
        }
        None => {
            increment_cpu_runnable_depth(target_cpu);
            let _ = RUNNABLE_TASK_CPU.insert(pid, target_cpu, 0);
        }
    }

    true
}

#[inline(always)]
pub(crate) fn mark_task_running(pid: u32, current_cpu: u32) {
    if pid == 0 {
        return;
    }

    if let Some(stored_cpu) = unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        let cpu_to_decrement = if valid_cpu(stored_cpu) {
            stored_cpu
        } else {
            current_cpu
        };
        decrement_cpu_runnable_depth(cpu_to_decrement);
        let _ = RUNNABLE_TASK_CPU.remove(pid);
    }
}

#[inline(always)]
pub(crate) fn remove_runnable_task_if_present(pid: u32) {
    if pid == 0 {
        return;
    }
    if let Some(cpu) = unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        decrement_cpu_runnable_depth(cpu);
        let _ = RUNNABLE_TASK_CPU.remove(pid);
    }
}

pub(crate) fn increment_target_pending(cpu: u32) {
    if !valid_cpu(cpu) {
        increment_drop_counter(DROP_CPU_ACCOUNTING_UNTRACKED);
        return;
    }
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only increment.
        unsafe { *depth = (*depth).saturating_add(1) };
    }
}

pub(crate) fn decrement_target_pending(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only decrement.
        unsafe { *depth = (*depth).saturating_sub(1) };
    }
}
