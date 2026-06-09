//! Pressure timeline report helpers.
//!
//! Owns pressure timeline windows, peak pressure windows, and pressure notes. Does not own data
//! quality, task row selection, clustering, diagnosis, or report orchestration.

use super::*;

pub(crate) fn build_pressure_timeline(
    intervals: &[IntervalRecord],
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> PressureTimelineSummary {
    if intervals.is_empty() {
        return PressureTimelineSummary {
            sample_count: 0,
            max_cpu_some: 0.0,
            max_mem_some: None,
            max_mem_full: None,
            max_io_some: None,
            max_io_full: None,
            windows: Vec::new(),
            peak_windows: Vec::new(),
            pressure_notes: vec![
                "No interval records loaded; pressure timeline unavailable".to_owned(),
            ],
            coverage: PressureTimelineCoverage::default(),
        };
    }

    let mut sorted_intervals = intervals.iter().collect::<Vec<_>>();
    sorted_intervals.sort_by_key(|record| record.elapsed_ms);

    let mut windows = Vec::with_capacity(sorted_intervals.len());
    let mut peak_windows = Vec::new();

    let mut max_cpu_some = 0.0_f64;
    let mut max_mem_some = 0.0_f64;
    let mut max_mem_full = 0.0_f64;
    let mut max_io_some = 0.0_f64;
    let mut max_io_full = 0.0_f64;

    let mut has_mem_psi = false;
    let mut has_io_psi = false;
    let mut has_near_spike_windows = false;

    for record in sorted_intervals {
        let near_spike = pressure_window_near_spike(record.elapsed_ms, clusters, cluster_window_ms);

        has_near_spike_windows |= near_spike;
        has_mem_psi |= record.mem_psi_some > 0.0 || record.mem_psi_full > 0.0;
        has_io_psi |= record.io_psi_some > 0.0 || record.io_psi_full > 0.0;

        max_cpu_some = max_cpu_some.max(record.cpu_psi_some);
        max_mem_some = max_mem_some.max(record.mem_psi_some);
        max_mem_full = max_mem_full.max(record.mem_psi_full);
        max_io_some = max_io_some.max(record.io_psi_some);
        max_io_full = max_io_full.max(record.io_psi_full);

        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::CpuSome,
                value: record.cpu_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemSome,
                value: record.mem_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemFull,
                value: record.mem_psi_full,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoSome,
                value: record.io_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoFull,
                value: record.io_psi_full,
                near_spike,
            },
        );

        windows.push(PressureWindow {
            elapsed_ms: record.elapsed_ms,
            cpu_some: record.cpu_psi_some,
            mem_some: Some(record.mem_psi_some),
            mem_full: Some(record.mem_psi_full),
            io_some: Some(record.io_psi_some),
            io_full: Some(record.io_psi_full),
            near_spike,
        });
    }

    peak_windows.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.elapsed_ms.cmp(&b.elapsed_ms))
    });
    peak_windows.truncate(MAX_PRESSURE_PEAK_WINDOWS);

    let max_mem_some = has_mem_psi.then_some(max_mem_some);
    let max_mem_full = has_mem_psi.then_some(max_mem_full);
    let max_io_some = has_io_psi.then_some(max_io_some);
    let max_io_full = has_io_psi.then_some(max_io_full);

    let coverage = PressureTimelineCoverage {
        interval_records_loaded: windows.len(),
        has_cpu_psi: true,
        has_mem_psi,
        has_io_psi,
        has_near_spike_windows,
    };

    let pressure_notes = build_pressure_notes(PressureNoteInput {
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        has_mem_psi,
        has_io_psi,
        peak_windows: &peak_windows,
    });

    PressureTimelineSummary {
        sample_count: windows.len(),
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        windows,
        peak_windows,
        pressure_notes,
        coverage,
    }
}

pub(crate) fn pressure_window_near_spike(
    elapsed_ms: u64,
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> bool {
    clusters.iter().any(|cluster| {
        cluster
            .points
            .iter()
            .filter_map(|point| point.elapsed_ms)
            .any(|cluster_elapsed_ms| elapsed_ms.abs_diff(cluster_elapsed_ms) <= cluster_window_ms)
    })
}

pub(crate) fn push_pressure_peak_window(
    peak_windows: &mut Vec<PressurePeakWindow>,
    peak_window: PressurePeakWindow,
) {
    if peak_window.value <= 0.0 {
        return;
    }

    peak_windows.push(peak_window);
}

pub(crate) struct PressureNoteInput<'a> {
    pub max_cpu_some: f64,
    pub max_mem_some: Option<f64>,
    pub max_mem_full: Option<f64>,
    pub max_io_some: Option<f64>,
    pub max_io_full: Option<f64>,
    pub has_mem_psi: bool,
    pub has_io_psi: bool,
    pub peak_windows: &'a [PressurePeakWindow],
}

pub(crate) fn build_pressure_notes(input: PressureNoteInput<'_>) -> Vec<String> {
    let max_cpu_some = input.max_cpu_some;
    let max_mem_some = input.max_mem_some;
    let max_mem_full = input.max_mem_full;
    let max_io_some = input.max_io_some;
    let max_io_full = input.max_io_full;
    let has_mem_psi = input.has_mem_psi;
    let has_io_psi = input.has_io_psi;
    let peak_windows = input.peak_windows;
    let mut notes = Vec::new();

    push_pressure_note_if_above(
        &mut notes,
        PressureKind::CpuSome,
        max_cpu_some,
        PRESSURE_NOTE_CPU_SOME,
        peak_windows,
        "CPU pressure",
    );

    if let Some(value) = max_mem_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemSome,
            value,
            PRESSURE_NOTE_MEM_SOME,
            peak_windows,
            "Memory pressure",
        );
    }
    if let Some(value) = max_mem_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemFull,
            value,
            PRESSURE_NOTE_MEM_FULL,
            peak_windows,
            "Memory full pressure",
        );
    }
    if let Some(value) = max_io_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoSome,
            value,
            PRESSURE_NOTE_IO_SOME,
            peak_windows,
            "I/O pressure",
        );
    }
    if let Some(value) = max_io_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoFull,
            value,
            PRESSURE_NOTE_IO_FULL,
            peak_windows,
            "I/O full pressure",
        );
    }

    if !has_mem_psi {
        notes.push("Memory PSI fields were not present in loaded intervals".to_owned());
    }
    if !has_io_psi {
        notes.push("I/O PSI fields were not present in loaded intervals".to_owned());
    }

    notes
}

pub(crate) fn push_pressure_note_if_above(
    notes: &mut Vec<String>,
    pressure_kind: PressureKind,
    value: f64,
    threshold: f64,
    peak_windows: &[PressurePeakWindow],
    label: &str,
) {
    if value < threshold {
        return;
    }

    let near_spike = peak_windows.iter().any(|peak_window| {
        pressure_kind_label(&peak_window.pressure_kind) == pressure_kind_label(&pressure_kind)
            && (peak_window.value - value).abs() <= f64::EPSILON
            && peak_window.near_spike
    });

    if near_spike {
        notes.push(format!(
            "{label} reached {:.1}% near a scheduler spike",
            value
        ));
    } else {
        notes.push(format!("{label} reached {:.1}%", value));
    }
}

pub(crate) fn pressure_kind_label(pressure_kind: &PressureKind) -> &'static str {
    match pressure_kind {
        PressureKind::CpuSome => "cpu_some",
        PressureKind::MemSome => "mem_some",
        PressureKind::MemFull => "mem_full",
        PressureKind::IoSome => "io_some",
        PressureKind::IoFull => "io_full",
    }
}
