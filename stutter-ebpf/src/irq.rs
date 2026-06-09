use aya_ebpf::{
    helpers::{bpf_get_smp_processor_id, bpf_ktime_get_ns},
    macros::map,
    maps::HashMap,
    programs::TracePointContext,
};
use stutter_common::{
    DROP_IRQ_START_TIMES_INSERT_FAILED, EVENT_IRQ_LATENCY, IrqEvent,
    tracepoint_offsets::IRQ_HANDLER_IRQ_OFFSET,
};

use crate::{
    drop_counters::increment_drop_counter,
    map_limits::{IRQ_START_TIMES_MAP_MAX_ENTRIES, TARGET_IRQS_MAP_MAX_ENTRIES},
    trace_read::read_i32,
};

#[map]
pub static TARGET_IRQS: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(TARGET_IRQS_MAP_MAX_ENTRIES, 0);

#[map]
pub static IRQ_START_TIMES: HashMap<u64, u64> =
    HashMap::<u64, u64>::with_max_entries(IRQ_START_TIMES_MAP_MAX_ENTRIES, 0);

#[inline(always)]
pub(crate) fn is_target_irq(irq: u32) -> bool {
    unsafe { TARGET_IRQS.get(irq).is_some() }
}

#[inline(always)]
pub(crate) fn irq_key(irq: u32, cpu: u32) -> u64 {
    // High 32 bits: IRQ number. Low 32 bits: CPU ID.
    // CPU IDs such as 65_536 remain entirely in the low half and do not collide
    // with the IRQ half of the packed key.
    ((irq as u64) << 32) | cpu as u64
}

#[inline(always)]
pub(crate) fn try_irq_handler_entry(ctx: TracePointContext) -> u32 {
    let mut irq: i32 = 0;
    if !read_i32(&ctx, IRQ_HANDLER_IRQ_OFFSET, &mut irq) {
        return 1;
    }
    if irq < 0 {
        return 0;
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return 0;
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let now = unsafe { bpf_ktime_get_ns() };
    if IRQ_START_TIMES.insert(key, now, 0).is_err() {
        increment_drop_counter(DROP_IRQ_START_TIMES_INSERT_FAILED);
    }
    0
}

#[inline(always)]
pub(crate) fn try_irq_handler_exit(ctx: TracePointContext) -> u32 {
    let mut irq: i32 = 0;
    if !read_i32(&ctx, IRQ_HANDLER_IRQ_OFFSET, &mut irq) {
        return 1;
    }
    if irq < 0 {
        return 0;
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return 0;
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let enter_ns = match unsafe { IRQ_START_TIMES.get(key) } {
        Some(ts) => *ts,
        None => return 0,
    };
    let _ = IRQ_START_TIMES.remove(key);

    let exit_ns = unsafe { bpf_ktime_get_ns() };
    let duration_ns = exit_ns.saturating_sub(enter_ns);

    emit_ringbuf_event!(
        IrqEvent,
        return 0,
        IrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq,
            cpu,
            _pad0: 0,
            enter_ns,
            exit_ns,
            duration_ns,
        }
    );
    0
}
