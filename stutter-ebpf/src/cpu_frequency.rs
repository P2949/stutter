use aya_ebpf::{helpers::bpf_ktime_get_ns, programs::TracePointContext};
use stutter_common::{
    CpuFreqEvent, EVENT_CPU_FREQ,
    tracepoint_offsets::{CPU_FREQUENCY_CPU_ID_OFFSET, CPU_FREQUENCY_STATE_OFFSET},
};

use crate::trace_read::read_u32;

#[inline(always)]
pub(crate) fn try_cpu_frequency(ctx: TracePointContext) -> u32 {
    let mut state: u32 = 0;
    if !read_u32(&ctx, CPU_FREQUENCY_STATE_OFFSET, &mut state) {
        return 1;
    }
    let mut cpu_id: u32 = 0;
    if !read_u32(&ctx, CPU_FREQUENCY_CPU_ID_OFFSET, &mut cpu_id) {
        return 1;
    }
    let now = unsafe { bpf_ktime_get_ns() };

    emit_ringbuf_event!(
        CpuFreqEvent,
        return 0,
        CpuFreqEvent {
            kind: EVENT_CPU_FREQ,
            cpu: cpu_id,
            state,
            _pad: 0,
            timestamp_ns: now,
        }
    );

    0
}
