//! Stutter changes affinity, nice, I/O priority and uclamp through native syscalls with readback,
//! never by running `taskset`, `nice`, `ionice`, `chrt` or `cpupower`, and it can never promote a
//! task to a real-time policy.
use std::{fs, path::Path};

use super::workspace_root;

fn production_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.is_dir() {
            if name != "tests" {
                production_files(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

#[test]
fn no_production_code_runs_a_tuning_binary_or_sets_a_real_time_policy() {
    let mut files = Vec::new();
    production_files(&workspace_root().join("stutter/src"), &mut files);
    assert!(files.len() > 300);
    let mut violations = Vec::new();
    for file in files {
        let path = file.to_string_lossy().into_owned();
        // This file and the architecture tests name the forbidden strings to forbid them.
        if path.contains("architecture_tests") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for binary in ["taskset", "nice", "ionice", "chrt", "cpupower", "renice"] {
                if line.contains(&format!("Command::new(\"{binary}\")")) {
                    violations.push(format!("{path}:{} runs `{binary}`", index + 1));
                }
            }
            if line.contains("sched_setscheduler") {
                violations.push(format!("{path}:{} calls sched_setscheduler", index + 1));
            }
            if line.contains("SYS_sched_setattr") && !path.ends_with("actions/syscalls.rs") {
                violations.push(format!(
                    "{path}:{} calls sched_setattr outside actions/syscalls.rs",
                    index + 1
                ));
            }
        }
    }
    // The one sched_setattr in the actions wrapper must keep the policy and the parameters.
    let wrapper = fs::read_to_string(workspace_root().join("stutter/src/actions/syscalls.rs"))
        .unwrap_or_default();
    for flag in ["SCHED_FLAG_KEEP_POLICY", "SCHED_FLAG_KEEP_PARAMS"] {
        if !wrapper.contains(flag) {
            violations.push(format!(
                "actions/syscalls.rs sched_setattr no longer sets {flag}"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "native-controls rule broken:\n{}",
        violations.join("\n")
    );
}
