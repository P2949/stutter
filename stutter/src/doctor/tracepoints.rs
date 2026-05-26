use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::{DoctorCheck, DoctorInput, DoctorStatus};
use crate::ebpf_loader;

const DEFAULT_TRACEPOINT_EVENTS_ROOT: &str = "/sys/kernel/tracing/events";

const STUTTER_TRACEPOINT_FORMATS: &[(&str, &str)] = &[
    ("sched/sched_wakeup", "sched/sched_wakeup/format"),
    ("sched/sched_wakeup_new", "sched/sched_wakeup_new/format"),
    ("sched/sched_switch", "sched/sched_switch/format"),
    (
        "sched/sched_migrate_task",
        "sched/sched_migrate_task/format",
    ),
    ("power/cpu_frequency", "power/cpu_frequency/format"),
    ("sched/sched_stat_wait", "sched/sched_stat_wait/format"),
    ("irq/irq_handler_entry", "irq/irq_handler_entry/format"),
    ("irq/irq_handler_exit", "irq/irq_handler_exit/format"),
    ("block/block_rq_issue", "block/block_rq_issue/format"),
    ("block/block_rq_complete", "block/block_rq_complete/format"),
    (
        "sched/sched_process_exit",
        "sched/sched_process_exit/format",
    ),
    (
        "sched/sched_process_exec",
        "sched/sched_process_exec/format",
    ),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointFormatDump {
    pub events_root: String,
    pub entries: Vec<TracepointFormatDumpEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointFormatDumpEntry {
    pub tracepoint: String,
    pub path: String,
    pub status: String,
    pub format: Option<String>,
    pub error: Option<String>,
}

pub(super) fn tracepoint_check(input: &DoctorInput) -> DoctorCheck {
    let report = ebpf_loader::tracepoint_preflight(
        Path::new(DEFAULT_TRACEPOINT_EVENTS_ROOT),
        true,
        false,
        input.irq_latency,
        input.block_io,
        true,
    );

    let mut details = BTreeMap::new();
    details.insert("sched_wakeup".to_owned(), report.sched_wakeup);
    details.insert("sched_switch".to_owned(), report.sched_switch);
    details.insert("sched_wakeup_new".to_owned(), report.sched_wakeup_new);
    details.insert(
        "sched_wakeup_new_coverage".to_owned(),
        report.sched_wakeup_new_coverage,
    );
    details.insert("sched_migrate_task".to_owned(), report.sched_migrate_task);
    details.insert("cpu_frequency".to_owned(), report.cpu_frequency);
    details.insert("sched_stat_wait".to_owned(), report.sched_stat_wait);
    details.insert("irq_handler".to_owned(), report.irq_handler);
    details.insert("block_rq".to_owned(), report.block_rq);
    details.insert(
        "block_io_correlation_basis".to_owned(),
        report.block_io_correlation_basis.clone(),
    );
    if !report.block_io_correlation_basis.is_empty() {
        details.insert(
            "block_io_correlation_confidence".to_owned(),
            ebpf_loader::BlockIoCorrelationBasis::from_str(&report.block_io_correlation_basis)
                .confidence()
                .to_owned(),
        );
    }
    for (idx, warning) in report.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }
    for (idx, error) in report.errors.iter().enumerate() {
        details.insert(format!("error_{idx}"), error.clone());
    }

    let status = if !report.errors.is_empty() {
        DoctorStatus::Fail
    } else if !report.warnings.is_empty() {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    DoctorCheck {
        name: "tracepoint_formats".to_owned(),
        status,
        message: match status {
            DoctorStatus::Pass => "required tracepoint formats look compatible".to_owned(),
            DoctorStatus::Warn => {
                "required tracepoints look usable, but optional probes may be degraded".to_owned()
            }
            DoctorStatus::Fail => {
                "required scheduler tracepoint formats are missing or incompatible".to_owned()
            }
        },
        details,
    }
}

pub(super) fn tracepoint_dump_command(json: bool) -> anyhow::Result<()> {
    let dump = build_tracepoint_format_dump(Path::new(DEFAULT_TRACEPOINT_EVENTS_ROOT));

    if json {
        println!("{}", serde_json::to_string_pretty(&dump)?);
    } else {
        print!("{}", render_tracepoint_format_dump_text(&dump));
    }

    Ok(())
}

pub(crate) fn build_tracepoint_format_dump(events_root: &Path) -> TracepointFormatDump {
    let entries = STUTTER_TRACEPOINT_FORMATS
        .iter()
        .map(|(tracepoint, relative_path)| {
            tracepoint_format_dump_entry(events_root, tracepoint, relative_path)
        })
        .collect();

    TracepointFormatDump {
        events_root: events_root.display().to_string(),
        entries,
    }
}

pub(crate) fn render_tracepoint_format_dump_text(dump: &TracepointFormatDump) -> String {
    let mut text = String::new();
    writeln!(&mut text, "stutter doctor tracepoints --dump").ok();
    writeln!(&mut text, "================================").ok();
    writeln!(&mut text, "events_root={}", dump.events_root).ok();
    writeln!(&mut text).ok();

    for entry in &dump.entries {
        writeln!(
            &mut text,
            "--- {} ({}) [{}]",
            entry.tracepoint, entry.path, entry.status
        )
        .ok();
        if let Some(format) = &entry.format {
            text.push_str(format);
            if !format.ends_with('\n') {
                text.push('\n');
            }
        } else {
            writeln!(
                &mut text,
                "error={}",
                entry.error.as_deref().unwrap_or("unknown")
            )
            .ok();
        }
        writeln!(&mut text).ok();
    }

    text
}

fn tracepoint_format_dump_entry(
    events_root: &Path,
    tracepoint: &str,
    relative_path: &str,
) -> TracepointFormatDumpEntry {
    let path = events_root.join(PathBuf::from(relative_path));

    match fs::read_to_string(&path) {
        Ok(format) => TracepointFormatDumpEntry {
            tracepoint: tracepoint.to_owned(),
            path: path.display().to_string(),
            status: "ok".to_owned(),
            format: Some(format),
            error: None,
        },
        Err(err) => TracepointFormatDumpEntry {
            tracepoint: tracepoint.to_owned(),
            path: path.display().to_string(),
            status: tracepoint_read_error_status(&err).to_owned(),
            format: None,
            error: Some(err.to_string()),
        },
    }
}

fn tracepoint_read_error_status(err: &io::Error) -> &'static str {
    if err.kind() == io::ErrorKind::NotFound {
        "missing"
    } else {
        "error"
    }
}
