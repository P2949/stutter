use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use stutter_common::tracepoint_offsets::{
    CPU_FREQUENCY_FIELDS, IRQ_HANDLER_FIELDS, SCHED_MIGRATE_TASK_FIELDS, SCHED_STAT_WAIT_FIELDS,
    SCHED_SWITCH_FIELDS, SCHED_WAKEUP_FIELDS, TRACEPOINT_CPU_FREQUENCY,
    TRACEPOINT_IRQ_HANDLER_ENTRY, TRACEPOINT_IRQ_HANDLER_EXIT, TRACEPOINT_SCHED_MIGRATE_TASK,
    TRACEPOINT_SCHED_PROCESS_EXEC, TRACEPOINT_SCHED_PROCESS_EXIT, TRACEPOINT_SCHED_STAT_WAIT,
    TRACEPOINT_SCHED_SWITCH, TRACEPOINT_SCHED_WAKEUP, TRACEPOINT_SCHED_WAKEUP_NEW,
    TracepointFieldSpec, TracepointName,
};

use super::{DoctorCheck, DoctorInput, DoctorStatus};
use crate::{ebpf::tracepoint_format::validate_tracepoint_format_named, ebpf_loader};

const DEFAULT_TRACEPOINT_EVENTS_ROOT: &str = "/sys/kernel/tracing/events";

struct TracepointDumpSpec {
    tracepoint: &'static str,
    relative_path: &'static str,
    validation_name: Option<TracepointName<'static>>,
    expected_fields: &'static [TracepointFieldSpec],
}

const STUTTER_TRACEPOINT_FORMATS: &[TracepointDumpSpec] = &[
    TracepointDumpSpec {
        tracepoint: "sched/sched_wakeup",
        relative_path: "sched/sched_wakeup/format",
        validation_name: Some(TRACEPOINT_SCHED_WAKEUP),
        expected_fields: SCHED_WAKEUP_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_wakeup_new",
        relative_path: "sched/sched_wakeup_new/format",
        validation_name: Some(TRACEPOINT_SCHED_WAKEUP_NEW),
        expected_fields: SCHED_WAKEUP_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_switch",
        relative_path: "sched/sched_switch/format",
        validation_name: Some(TRACEPOINT_SCHED_SWITCH),
        expected_fields: SCHED_SWITCH_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_migrate_task",
        relative_path: "sched/sched_migrate_task/format",
        validation_name: Some(TRACEPOINT_SCHED_MIGRATE_TASK),
        expected_fields: SCHED_MIGRATE_TASK_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "power/cpu_frequency",
        relative_path: "power/cpu_frequency/format",
        validation_name: Some(TRACEPOINT_CPU_FREQUENCY),
        expected_fields: CPU_FREQUENCY_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_stat_wait",
        relative_path: "sched/sched_stat_wait/format",
        validation_name: Some(TRACEPOINT_SCHED_STAT_WAIT),
        expected_fields: SCHED_STAT_WAIT_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "irq/irq_handler_entry",
        relative_path: "irq/irq_handler_entry/format",
        validation_name: Some(TRACEPOINT_IRQ_HANDLER_ENTRY),
        expected_fields: IRQ_HANDLER_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "irq/irq_handler_exit",
        relative_path: "irq/irq_handler_exit/format",
        validation_name: Some(TRACEPOINT_IRQ_HANDLER_EXIT),
        expected_fields: IRQ_HANDLER_FIELDS,
    },
    TracepointDumpSpec {
        tracepoint: "block/block_rq_issue",
        relative_path: "block/block_rq_issue/format",
        validation_name: None,
        expected_fields: &[],
    },
    TracepointDumpSpec {
        tracepoint: "block/block_rq_complete",
        relative_path: "block/block_rq_complete/format",
        validation_name: None,
        expected_fields: &[],
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_process_exit",
        relative_path: "sched/sched_process_exit/format",
        validation_name: Some(TRACEPOINT_SCHED_PROCESS_EXIT),
        expected_fields: &[],
    },
    TracepointDumpSpec {
        tracepoint: "sched/sched_process_exec",
        relative_path: "sched/sched_process_exec/format",
        validation_name: Some(TRACEPOINT_SCHED_PROCESS_EXEC),
        expected_fields: &[],
    },
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointFormatDump {
    pub events_root: String,
    pub kernel_osrelease: Option<String>,
    pub kernel_version: Option<String>,
    pub entries: Vec<TracepointFormatDumpEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointFormatDumpEntry {
    pub tracepoint: String,
    pub path: String,
    pub status: String,
    pub format: Option<String>,
    pub error: Option<String>,
    pub validation: TracepointFormatValidationDump,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointFormatValidationDump {
    pub status: String,
    pub expected_fields: Vec<TracepointExpectedFieldDump>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TracepointExpectedFieldDump {
    pub name: String,
    pub offset: usize,
}

pub(super) fn tracepoint_check(input: &DoctorInput) -> DoctorCheck {
    let report = ebpf_loader::tracepoint_preflight(
        tracepoint_events_root(input),
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

pub(super) fn tracepoint_dump_command(input: &DoctorInput) -> anyhow::Result<()> {
    let dump = build_tracepoint_format_dump(tracepoint_events_root(input));

    if input.json {
        println!("{}", serde_json::to_string_pretty(&dump)?);
    } else {
        print!("{}", render_tracepoint_format_dump_text(&dump));
    }

    Ok(())
}

pub(crate) fn build_tracepoint_format_dump(events_root: &Path) -> TracepointFormatDump {
    let entries = STUTTER_TRACEPOINT_FORMATS
        .iter()
        .map(|spec| tracepoint_format_dump_entry(events_root, spec))
        .collect();

    TracepointFormatDump {
        events_root: events_root.display().to_string(),
        kernel_osrelease: read_optional_trimmed("/proc/sys/kernel/osrelease"),
        kernel_version: read_optional_trimmed("/proc/version"),
        entries,
    }
}

pub(crate) fn render_tracepoint_format_dump_text(dump: &TracepointFormatDump) -> String {
    let mut text = String::new();
    writeln!(&mut text, "stutter doctor tracepoints --dump").ok();
    writeln!(&mut text, "================================").ok();
    writeln!(&mut text, "events_root={}", dump.events_root).ok();
    if let Some(kernel) = &dump.kernel_osrelease {
        writeln!(&mut text, "kernel_osrelease={kernel}").ok();
    }
    if let Some(version) = &dump.kernel_version {
        writeln!(&mut text, "kernel_version={version}").ok();
    }
    writeln!(
        &mut text,
        "bug_report_hint=Attach this output from `stutter doctor tracepoints --dump --json` when reporting tracepoint compatibility issues."
    )
    .ok();
    writeln!(&mut text).ok();

    for entry in &dump.entries {
        writeln!(
            &mut text,
            "--- {} ({}) [{}]",
            entry.tracepoint, entry.path, entry.status
        )
        .ok();
        writeln!(&mut text, "validation={}", entry.validation.status).ok();

        if !entry.validation.expected_fields.is_empty() {
            let expected = entry
                .validation
                .expected_fields
                .iter()
                .map(|field| format!("{}@{}", field.name, field.offset))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(&mut text, "expected_fields={expected}").ok();
        }

        if let Some(error) = &entry.validation.error {
            writeln!(&mut text, "validation_error={error}").ok();
        }

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

fn tracepoint_events_root(input: &DoctorInput) -> &Path {
    input
        .tracepoint_events_root
        .as_deref()
        .unwrap_or_else(|| Path::new(DEFAULT_TRACEPOINT_EVENTS_ROOT))
}

fn tracepoint_format_dump_entry(
    events_root: &Path,
    spec: &TracepointDumpSpec,
) -> TracepointFormatDumpEntry {
    let path = events_root.join(PathBuf::from(spec.relative_path));

    match fs::read_to_string(&path) {
        Ok(format) => TracepointFormatDumpEntry {
            tracepoint: spec.tracepoint.to_owned(),
            path: path.display().to_string(),
            status: "ok".to_owned(),
            validation: validate_dumped_tracepoint_format(spec, &format),
            format: Some(format),
            error: None,
        },
        Err(err) => TracepointFormatDumpEntry {
            tracepoint: spec.tracepoint.to_owned(),
            path: path.display().to_string(),
            status: tracepoint_read_error_status(&err).to_owned(),
            validation: TracepointFormatValidationDump {
                status: "unavailable".to_owned(),
                expected_fields: expected_field_dump(spec.expected_fields),
                error: None,
            },
            format: None,
            error: Some(err.to_string()),
        },
    }
}

fn expected_field_dump(fields: &[TracepointFieldSpec]) -> Vec<TracepointExpectedFieldDump> {
    fields
        .iter()
        .map(|field| TracepointExpectedFieldDump {
            name: field.name.as_str().to_owned(),
            offset: field.offset,
        })
        .collect()
}

fn validate_dumped_tracepoint_format(
    spec: &TracepointDumpSpec,
    format: &str,
) -> TracepointFormatValidationDump {
    let expected_fields = expected_field_dump(spec.expected_fields);

    let Some(name) = spec.validation_name else {
        return TracepointFormatValidationDump {
            status: "not_checked".to_owned(),
            expected_fields,
            error: None,
        };
    };

    if spec.expected_fields.is_empty() {
        return TracepointFormatValidationDump {
            status: "not_checked".to_owned(),
            expected_fields,
            error: None,
        };
    }

    match validate_tracepoint_format_named(name, format, spec.expected_fields) {
        Ok(()) => TracepointFormatValidationDump {
            status: "compatible".to_owned(),
            expected_fields,
            error: None,
        },
        Err(err) => TracepointFormatValidationDump {
            status: "mismatch".to_owned(),
            expected_fields,
            error: Some(format!("{err:#}")),
        },
    }
}

fn read_optional_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn tracepoint_read_error_status(err: &io::Error) -> &'static str {
    if err.kind() == io::ErrorKind::NotFound {
        "missing"
    } else {
        "error"
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{build_tracepoint_format_dump, render_tracepoint_format_dump_text};

    fn write_format(events_root: &Path, relative: &str, contents: &str) {
        let path = events_root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn tracepoint_dump_reports_validation_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        write_format(
            root,
            "sched/sched_switch/format",
            "field:pid_t prev_pid; offset:24; size:4; signed:1;\n\
             field:long prev_state; offset:32; size:8; signed:1;\n\
             field:char next_comm[16]; offset:40; size:16; signed:1;\n\
             field:pid_t next_pid; offset:52; size:4; signed:1;\n\
             field:int next_prio; offset:60; size:4; signed:1;\n",
        );

        let dump = build_tracepoint_format_dump(root);
        let sched_switch = dump
            .entries
            .iter()
            .find(|entry| entry.tracepoint == "sched/sched_switch")
            .expect("sched_switch dump entry");

        assert_eq!(sched_switch.status, "ok");
        assert_eq!(sched_switch.validation.status, "mismatch");
        assert!(
            sched_switch
                .validation
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("expected offset 56"),
            "unexpected validation error: {sched_switch:#?}"
        );
    }

    #[test]
    fn tracepoint_dump_text_includes_bug_report_and_validation_context() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        write_format(
            root,
            "sched/sched_wakeup/format",
            "field:pid_t pid; offset:24; size:4; signed:1;\n\
             field:int prio; offset:28; size:4; signed:1;\n\
             field:int target_cpu; offset:32; size:4; signed:1;\n",
        );

        let dump = build_tracepoint_format_dump(root);
        let text = render_tracepoint_format_dump_text(&dump);

        assert!(text.contains("bug_report_hint="));
        assert!(text.contains("validation=compatible"));
        assert!(text.contains("expected_fields=pid@24, prio@28, target_cpu@32"));
    }
}
