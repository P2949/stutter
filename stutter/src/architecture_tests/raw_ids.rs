//! Architecture guard against new raw task/process/CPU/IRQ identity fields.

use std::{fs, path::Path};

use super::{
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

const RAW_ID_FIELD_NAMES: &[&str] = &["pid", "tid", "process_pid", "task_tid", "cpu", "irq"];

#[derive(Clone, Copy, Debug)]
struct RawIdAllowedPath {
    path: &'static str,
    reason: &'static str,
}

const RAW_ID_ALLOWED_PATHS: &[RawIdAllowedPath] = &[
    RawIdAllowedPath {
        path: "src/actions/cgroup/model.rs",
        reason: "cgroup restore snapshots preserve numeric task identity at the action boundary",
    },
    RawIdAllowedPath {
        path: "src/actions/cpu_power.rs",
        reason: "CPU sysfs provider boundary uses kernel CPU numbers",
    },
    RawIdAllowedPath {
        path: "src/actions/ioprio/model.rs",
        reason: "Ioprio action model preserves numeric task identity at the action boundary",
    },
    RawIdAllowedPath {
        path: "src/actions/uclamp/models.rs",
        reason: "Uclamp action model preserves numeric task identity at the action boundary",
    },
    RawIdAllowedPath {
        path: "src/actions/irq_affinity/model.rs",
        reason: "IRQ affinity action DTOs preserve kernel IRQ numbers",
    },
    RawIdAllowedPath {
        path: "src/actions/model.rs",
        reason: "serialized action compatibility constructors still accept legacy numeric IDs",
    },
    RawIdAllowedPath {
        path: "src/advisor/models.rs",
        reason: "advisor evidence DTOs preserve kernel IRQ numbers for observe-only diagnostics",
    },
    RawIdAllowedPath {
        path: "src/alert.rs",
        reason: "alert payloads are external DTOs with stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/autotune/baseline.rs",
        reason: "baseline identity keys retain persisted numeric compatibility",
    },
    RawIdAllowedPath {
        path: "src/autotune/providers/irq_affinity/model.rs",
        reason: "IRQ provider evidence mirrors kernel IRQ numbering",
    },
    RawIdAllowedPath {
        path: "src/actions/nice/model.rs",
        reason: "Nice action model preserves numeric task identity at the action boundary",
    },
    RawIdAllowedPath {
        path: "src/autotune/status/model.rs",
        reason: "daemon status DTOs expose numeric process IDs",
    },
    RawIdAllowedPath {
        path: "src/display_topology.rs",
        reason: "display topology DTOs expose external process IDs",
    },
    RawIdAllowedPath {
        path: "src/events/domain.rs",
        reason: "decoded event boundary mirrors raw eBPF CPU/IRQ fields",
    },
    RawIdAllowedPath {
        path: "src/focus/snapshot.rs",
        reason: "focus process snapshots mirror procfs IDs before focus grouping",
    },
    RawIdAllowedPath {
        path: "src/foreground/model.rs",
        reason: "foreground provider snapshots preserve external compositor/window PIDs",
    },
    RawIdAllowedPath {
        path: "src/foreground/parse/x11.rs",
        reason: "X11 parser boundary reads raw window PID properties",
    },
    RawIdAllowedPath {
        path: "src/metadata.rs",
        reason: "metadata spike DTOs preserve numeric CPU fields",
    },
    RawIdAllowedPath {
        path: "src/metrics/interval.rs",
        reason: "interval records are persisted DTOs with stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/otel.rs",
        reason: "OpenTelemetry event payloads emit numeric attributes",
    },
    RawIdAllowedPath {
        path: "src/process/model.rs",
        reason: "ProcInfo mirrors procfs raw process IDs before typed TaskInfo conversion",
    },
    RawIdAllowedPath {
        path: "src/recorder/event_types.rs",
        reason: "recorded event DTOs preserve stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/recorder/session_files.rs",
        reason: "session artifact DTOs preserve stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/report/model.rs",
        reason: "report model DTOs preserve stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/session_events.rs",
        reason: "session event compatibility payloads expose numeric process/task IDs",
    },
    RawIdAllowedPath {
        path: "src/spike.rs",
        reason: "spike analysis DTOs preserve stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/summary/model.rs",
        reason: "summary DTOs preserve stable numeric JSON fields",
    },
    RawIdAllowedPath {
        path: "src/topology.rs",
        reason: "CPU topology DTOs expose kernel CPU numbers",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawIdField {
    path: String,
    line_number: usize,
    line: String,
}

fn raw_id_fields_in_file(path: &Path) -> Vec<RawIdField> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    raw_id_fields_in_source(&source, &relative_path)
}

fn raw_id_fields_in_source(source: &str, path: &str) -> Vec<RawIdField> {
    if is_test_or_fixture_path(path) || path_is_allowed(path) {
        return Vec::new();
    }

    let mut fields = Vec::new();
    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        let trimmed = line.trim();
        if !looks_like_public_field(trimmed) {
            continue;
        }

        if RAW_ID_FIELD_NAMES
            .iter()
            .any(|field_name| raw_id_field_matches(trimmed, field_name))
        {
            fields.push(RawIdField {
                path: path.to_owned(),
                line_number,
                line: trimmed.to_owned(),
            });
        }
    }

    fields
}

fn looks_like_public_field(line: &str) -> bool {
    (line.starts_with("pub ") || line.starts_with("pub(")) && !line.contains(" fn ")
}

fn raw_id_field_matches(line: &str, field_name: &str) -> bool {
    let Some((left, right)) = line.split_once(':') else {
        return false;
    };
    let Some(name) = left.split_whitespace().last() else {
        return false;
    };
    let ty = right.trim().trim_end_matches(',').trim();
    name == field_name && (ty == "u32" || ty == "Option<u32>")
}

fn path_is_allowed(path: &str) -> bool {
    RAW_ID_ALLOWED_PATHS.iter().any(|allowed| {
        if allowed.path.ends_with('/') {
            path.starts_with(allowed.path)
        } else {
            path == allowed.path
        }
    })
}

fn is_test_or_fixture_path(path: &str) -> bool {
    path == "src/architecture_tests.rs"
        || path == "src/artifact_contract_tests.rs"
        || path == "src/recording_fixture_tests.rs"
        || path == "src/test_fixture_builder.rs"
        || path == "src/test_support.rs"
        || path.ends_with("/test_support.rs")
        || path.contains("/architecture_tests/")
        || path.contains("/planner_tests/")
        || path.contains("/regression_tests/")
        || path.contains("/test_fixture_builder/")
        || path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs")
}

#[test]
fn raw_id_field_scanner_catches_public_raw_identity_fields() {
    let source = r#"
pub struct RuntimeModel {
    pub tid: u32,
    pub process_pid: Option<u32>,
    pub unrelated_count: u32,
}

#[cfg(test)]
mod tests {
    pub struct Fixture {
        pub tid: u32,
    }
}
"#;

    assert_eq!(
        raw_id_fields_in_source(source, "src/runtime_model.rs"),
        vec![
            RawIdField {
                path: "src/runtime_model.rs".to_owned(),
                line_number: 3,
                line: "pub tid: u32,".to_owned(),
            },
            RawIdField {
                path: "src/runtime_model.rs".to_owned(),
                line_number: 4,
                line: "pub process_pid: Option<u32>,".to_owned(),
            },
        ]
    );
    assert!(raw_id_fields_in_source(source, "src/metrics/interval.rs").is_empty());
}

#[test]
fn new_raw_identity_fields_require_boundary_exceptions() {
    for allowed in RAW_ID_ALLOWED_PATHS {
        assert!(
            !allowed.reason.trim().is_empty(),
            "raw ID allowlist entry '{}' must have a reason",
            allowed.path
        );
    }

    let mut violations = Vec::new();

    for file in rust_files_under(&crate_src_root()) {
        for field in raw_id_fields_in_file(&file) {
            violations.push(format!(
                "{}:{} raw identity field needs a typed ID or explicit ABI/DTO boundary exception: {}",
                field.path, field.line_number, field.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "raw identity field guard failed:\n{}",
        violations.join("\n")
    );
}
