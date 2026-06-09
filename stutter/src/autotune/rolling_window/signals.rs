use std::collections::BTreeSet;

use super::{
    RollingWindow,
    utils::{is_compile_progress_record, overlap_basis_label},
};
use crate::autotune::objective::{
    ObjectiveSignalQuality, ObjectiveSignalQualitySnapshot, ObjectiveSignals,
};

const GPU_THERMAL_DEGRADED_MILLIDEGREES: u32 = 85_000;
const GPU_POWER_LIMIT_BUSY_PERCENT: u32 = 95;
const GPU_POWER_LIMIT_LOW_CLOCK_MHZ: u32 = 300;

pub(crate) fn compute_objective_signals(window: &RollingWindow) -> ObjectiveSignals {
    let block_io_overlap_count = window
        .block_io_events()
        .iter()
        .filter(|event| event.duration_ns > 0)
        .count() as u64;
    let block_io_worst_latency_ns = window
        .block_io_events()
        .iter()
        .map(|event| event.duration_ns)
        .max()
        .unwrap_or(0);
    let dirty_writeback_events = window
        .block_io_events()
        .iter()
        .filter(|event| event.rwbs.contains('W') || event.rwbs.contains('F'))
        .count() as u64;
    let block_io_overlap_basis = overlap_basis_label(
        window
            .block_io_events()
            .iter()
            .map(|event| event.correlation_basis.as_ref()),
    );
    let block_io_quality = source_quality_for_block_io_basis(block_io_overlap_basis.as_deref());
    let has_block_io_events = !window.block_io_events().is_empty();

    let irq_worst_event = window
        .irq_events()
        .iter()
        .filter(|event| event.duration_ns > 0)
        .max_by_key(|event| event.duration_ns);
    let irq_overlap_count = window
        .irq_events()
        .iter()
        .filter(|event| event.duration_ns > 0)
        .count() as u64;
    let irq_worst_overlap_ns = irq_worst_event.map(|event| event.duration_ns).unwrap_or(0);
    let has_irq_events = !window.irq_events().is_empty();

    let thermal_samples = window
        .gpu_samples()
        .iter()
        .filter_map(|sample| sample.temp_millidegrees)
        .collect::<Vec<_>>();
    let thermal_throttle_count = thermal_samples
        .iter()
        .filter(|temp| **temp >= GPU_THERMAL_DEGRADED_MILLIDEGREES)
        .count() as u64;
    let thermal_degraded = (!thermal_samples.is_empty()).then_some(thermal_throttle_count > 0);

    let cpu_power_limited_event = window
        .cpu_freq_events()
        .iter()
        .find(|event| event.freq_khz == 0);
    let cpu_power_limited =
        (!window.cpu_freq_events().is_empty()).then_some(cpu_power_limited_event.is_some());

    let latest_gpu = window.gpu_samples().back();
    let gpu_power_limited_event = window.gpu_samples().iter().find(|sample| {
        sample.gpu_busy_percent.unwrap_or(0) >= GPU_POWER_LIMIT_BUSY_PERCENT
            && sample.gpu_clock_mhz.unwrap_or(u32::MAX) <= GPU_POWER_LIMIT_LOW_CLOCK_MHZ
    });
    let gpu_power_limited =
        (!window.gpu_samples().is_empty()).then_some(gpu_power_limited_event.is_some());
    let gpu_power_limit_reason = gpu_power_limited_event
        .and_then(|sample| sample.power_limit_reason.clone())
        .or_else(|| {
            gpu_power_limited_event
                .is_some()
                .then(|| "busy_high_clock_low".to_owned())
        });
    let gpu_power_quality = if window.gpu_samples().iter().any(|sample| {
        sample.gpu_busy_percent.is_some()
            || sample.gpu_clock_mhz.is_some()
            || sample.temp_millidegrees.is_some()
            || sample.power_microwatts.is_some()
    }) {
        ObjectiveSignalQuality::Direct
    } else {
        ObjectiveSignalQuality::Missing
    };

    let memory_pressure_some_avg10_percent = (!window.intervals().is_empty()).then(|| {
        let total = window
            .intervals()
            .iter()
            .map(|record| record.mem_psi_some.max(0.0))
            .sum::<f64>();
        (total / window.intervals().len() as f64) as f32
    });
    let swap_activity_events = (!window.intervals().is_empty()).then(|| {
        window
            .intervals()
            .iter()
            .map(|record| record.major_faults)
            .fold(0_u64, u64::saturating_add)
    });
    let mem_stall_spike_count = (!window.intervals().is_empty()).then(|| {
        window
            .intervals()
            .iter()
            .map(|record| u64::from(record.mem_psi_spike))
            .fold(0_u64, u64::saturating_add)
    });
    let compile_progress_elapsed = window
        .intervals()
        .iter()
        .filter(|record| is_compile_progress_record(record))
        .map(|record| record.elapsed_ms)
        .collect::<BTreeSet<_>>();
    let compile_progress_intervals = compile_progress_elapsed.len() as u64;
    let compile_progress_samples = window
        .intervals()
        .iter()
        .filter(|record| is_compile_progress_record(record))
        .map(|record| record.samples)
        .fold(0_u64, u64::saturating_add);
    let has_compile_progress = compile_progress_intervals > 0;

    let gpu_active_render_node = latest_gpu.and_then(|sample| sample.render_node.clone());
    let gpu_drm_card = latest_gpu.and_then(|sample| sample.drm_card.clone());

    let signal_quality = ObjectiveSignalQualitySnapshot {
        block_io_overlap: block_io_quality,
        irq_overlap: if has_irq_events {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
        thermal: if thermal_degraded.is_some() {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
        cpu_power: if cpu_power_limited.is_some() {
            ObjectiveSignalQuality::Derived
        } else {
            ObjectiveSignalQuality::Missing
        },
        gpu_power: gpu_power_quality,
        gpu_active_render_node: if gpu_active_render_node.is_some() {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
        memory_pressure: if memory_pressure_some_avg10_percent.is_some() {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
        swap_activity: if swap_activity_events.is_some() {
            ObjectiveSignalQuality::Approximate
        } else {
            ObjectiveSignalQuality::Missing
        },
        dirty_writeback: if has_block_io_events {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
        frame_pacing: if window.frames().is_empty() {
            ObjectiveSignalQuality::Missing
        } else {
            ObjectiveSignalQuality::Direct
        },
        foreground_latency: if window.intervals().is_empty() {
            ObjectiveSignalQuality::Missing
        } else {
            ObjectiveSignalQuality::Derived
        },
        compile_throughput: if has_compile_progress {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        },
    };

    let irq_quality = if has_irq_events {
        ObjectiveSignalQuality::Direct
    } else {
        ObjectiveSignalQuality::Missing
    };
    let block_io_overlap_trust = has_block_io_events.then(|| block_io_quality.as_str().to_owned());
    let irq_overlap_trust = has_irq_events.then(|| irq_quality.as_str().to_owned());
    let irq_overlap_basis = has_irq_events.then(|| "irq-duration".to_owned());

    ObjectiveSignals {
        block_io_overlap_count: has_block_io_events.then_some(block_io_overlap_count),
        block_io_worst_latency_ns: has_block_io_events.then_some(block_io_worst_latency_ns),
        block_io_overlap_basis,
        block_io_overlap_trust,
        irq_overlap_count: has_irq_events.then_some(irq_overlap_count),
        irq_worst_overlap_ns: has_irq_events.then_some(irq_worst_overlap_ns),
        irq_hot_irq: irq_worst_event.map(|event| event.irq),
        irq_hot_cpu: irq_worst_event.map(|event| event.cpu),
        irq_overlap_basis,
        irq_overlap_trust,
        thermal_degraded,
        thermal_throttle_count: thermal_degraded.map(|_| thermal_throttle_count),
        cpu_power_limited,
        cpu_power_limited_cpu: cpu_power_limited_event.map(|event| event.cpu),
        cpu_power_limit_source: cpu_power_limited_event.map(|_| "cpu_freq_zero_khz".to_owned()),
        cpu_power_limited_policy: cpu_power_limited_event.map(|event| format!("cpu{}", event.cpu)),
        gpu_power_limited,
        gpu_power_limit_reason,
        gpu_busy_percent: latest_gpu.and_then(|sample| sample.gpu_busy_percent),
        gpu_clock_mhz: latest_gpu.and_then(|sample| sample.gpu_clock_mhz),
        gpu_temp_millidegrees: latest_gpu.and_then(|sample| sample.temp_millidegrees),
        gpu_drm_card,
        gpu_active_render_node,
        gpu_focus_confidence: latest_gpu
            .and_then(|sample| sample.render_node.as_ref())
            .map(|_| 0.85),
        gpu_focus_source: latest_gpu
            .and_then(|sample| sample.render_node.as_ref())
            .map(|_| "gpu_sample".to_owned()),
        memory_pressure_some_avg10_percent,
        swap_activity_events,
        mem_stall_spike_count,
        dirty_writeback_events: has_block_io_events.then_some(dirty_writeback_events),
        frame_p99_ms: Some(window.frame_p99_ms()),
        foreground_over_5ms: Some(
            window
                .intervals()
                .iter()
                .map(|record| record.over_5ms)
                .fold(0_u64, u64::saturating_add),
        ),
        compile_progress_intervals: has_compile_progress.then_some(compile_progress_intervals),
        compile_progress_samples: has_compile_progress.then_some(compile_progress_samples),
        compile_progress_source: has_compile_progress
            .then(|| "build-compiler-linker-intervals".to_owned()),
        signal_quality,
    }
}

fn source_quality_for_block_io_basis(basis: Option<&str>) -> ObjectiveSignalQuality {
    match basis {
        Some("request-pointer") => ObjectiveSignalQuality::Direct,
        Some(_) => ObjectiveSignalQuality::Approximate,
        None => ObjectiveSignalQuality::Missing,
    }
}
