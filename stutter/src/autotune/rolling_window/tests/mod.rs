use super::*;
use crate::{
    autotune::{
        objective::ObjectiveSignalQuality,
        quality::{OnlineDataQuality, OnlineDataQualityPolicy},
    },
    diagnosis::{Confidence, LiveDiagnosisEntry, StutterCause},
    process_tree::TaskClass,
};

fn interval(elapsed_ms: u64, samples: u64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        samples,
        ..Default::default()
    }
}

fn frame(elapsed_ms: u64, frametime_ms: f64) -> FrameEvent {
    FrameEvent {
        elapsed_ms,
        frametime_ms,
    }
}

fn irq_event(elapsed_ms: u64, duration_ns: u64) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: Some(elapsed_ms),
        irq: 44,
        cpu: 2,
        enter_ns: 1_000,
        exit_ns: 1_000 + duration_ns,
        duration_ns,
    }
}

fn block_io_event(elapsed_ms: u64, duration_ns: u64) -> BlockIoRecord {
    BlockIoRecord {
        elapsed_ms,
        tid: 77.into(),
        dev: 1,
        nr_sector: 8,
        correlation_basis: "dev-sector".into(),
        sector: 99,
        duration_ns,
        timestamp_ns: 2_000 + duration_ns,
        rwbs: "R".to_owned(),
    }
}

fn gpu_sample(elapsed_ms: u64, temp_millidegrees: u32) -> GpuSample {
    GpuSample {
        elapsed_ms,
        temp_millidegrees: Some(temp_millidegrees),
        gpu_busy_percent: Some(96),
        gpu_clock_mhz: Some(250),
        ..GpuSample::default()
    }
}

fn diagnosis(elapsed_ms: u64, cause: StutterCause) -> LiveDiagnosisEntry {
    LiveDiagnosisEntry {
        elapsed_ms,
        cause,
        confidence: Confidence::Medium,
        anchor_class: TaskClass::Game,
        anchor_comm: "RenderThread".to_owned(),
        evidence: vec!["test evidence".to_owned()],
    }
}
fn assert_non_decreasing<T, F>(items: &VecDeque<T>, elapsed_ms: F)
where
    F: Fn(&T) -> u64,
{
    let mut previous = None;
    for item in items {
        let current = elapsed_ms(item);
        if let Some(previous) = previous {
            assert!(previous <= current);
        }
        previous = Some(current);
    }
}

mod pruning;

mod irq_ingestion;

mod scoring;

mod signals;

mod out_of_order;
