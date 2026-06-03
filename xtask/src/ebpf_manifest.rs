use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use anyhow::{Context, bail};

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const EVENT_ABI_VERSION: u32 = 1;
const MAP_NAMES: &[&str] = &[
    "BLOCK_START",
    "CPU_RUNNABLE_DEPTH",
    "DROP_COUNTERS",
    "EVENTS",
    "FENCE_SIGNAL_TIMES",
    "FENCE_WAIT_STARTS",
    "IRQ_START_TIMES",
    "KMS_FLIP_STARTS",
    "PREV_FAULTS",
    "RUNNABLE_TASK_CPU",
    "TARGET_CGROUP_IDS",
    "TARGET_IRQS",
    "TARGET_PENDING_WAKEUPS",
    "TARGET_PIDS",
    "WAKEUP_CONSUMED",
    "WAKEUP_DATA",
    "WAKEUP_SEQ",
];
const PROGRAM_NAMES: &[&str] = &[
    "amdgpu_flip_done",
    "amdgpu_flip_request",
    "amdgpu_vblank_event",
    "block_rq_complete",
    "block_rq_issue",
    "cpu_frequency",
    "drm_fence_signal",
    "drm_fence_wait_done",
    "drm_fence_wait_start",
    "drm_flip_done",
    "drm_flip_request",
    "drm_vblank_event",
    "i915_flip_done",
    "i915_flip_request",
    "irq_handler_entry",
    "irq_handler_exit",
    "major_fault",
    "minor_fault",
    "sched_migrate_task",
    "sched_process_exec",
    "sched_process_exit",
    "sched_stat_wait",
    "sched_switch",
    "sched_wakeup",
    "sched_wakeup_new",
];

pub fn run_ebpf_manifest(root: &Path, object: &Path, out: &Path) -> anyhow::Result<()> {
    let metadata = fs::metadata(object)
        .with_context(|| format!("failed to stat eBPF object {}", object.display()))?;
    if !metadata.is_file() {
        bail!("eBPF object is not a regular file: {}", object.display());
    }
    if metadata.len() == 0 {
        bail!("eBPF object is empty: {}", object.display());
    }

    let sha256 = sha256sum(object)?;
    let version = stutter_version(root)?;
    let json = render_ebpf_manifest_json(&version, &sha256);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, json).with_context(|| format!("failed to write {}", out.display()))?;
    println!("ebpf_manifest={}", out.display());
    Ok(())
}

fn stutter_version(root: &Path) -> anyhow::Result<String> {
    let manifest = root.join("stutter/Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read {}", manifest.display()))?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "version"
        {
            return Ok(value.trim().trim_matches('"').to_owned());
        }
    }
    bail!(
        "failed to find stutter package version in {}",
        manifest.display()
    );
}

fn sha256sum(path: &Path) -> anyhow::Result<String> {
    let output = Command::new("sha256sum").arg(path).output().or_else(|_| {
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
    })?;
    parse_sha256_output(path, output)
}

fn parse_sha256_output(path: &Path, output: Output) -> anyhow::Result<String> {
    if !output.status.success() {
        bail!(
            "sha256 command failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("sha256 output was not UTF-8")?;
    let hash = stdout
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("sha256 command produced no hash for {}", path.display()))?;
    if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!(
            "sha256 command produced invalid hash for {}",
            path.display()
        );
    }
    Ok(hash.to_ascii_lowercase())
}

fn render_ebpf_manifest_json(stutter_version: &str, sha256: &str) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schema_version\": {},\n",
            "  \"stutter_version\": \"{}\",\n",
            "  \"ebpf_object_sha256\": \"{}\",\n",
            "  \"event_abi_version\": {},\n",
            "  \"map_names\": {},\n",
            "  \"program_names\": {}\n",
            "}}\n"
        ),
        MANIFEST_SCHEMA_VERSION,
        escape_json(stutter_version),
        escape_json(sha256),
        EVENT_ABI_VERSION,
        render_json_string_array(MAP_NAMES),
        render_json_string_array(PROGRAM_NAMES),
    )
}

fn render_json_string_array(values: &[&str]) -> String {
    let joined = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{joined}]")
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_manifest_with_hash_maps_and_programs() {
        let json = render_ebpf_manifest_json(
            "0.1.0",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );

        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"event_abi_version\": 1"));
        assert!(json.contains("\"ebpf_object_sha256\""));
        assert!(json.contains("\"EVENTS\""));
        assert!(json.contains("\"sched_switch\""));
        assert!(json.contains("\"drm_fence_signal\""));
    }

    #[test]
    fn rejects_malformed_sha256_output() {
        let output = Output {
            status: exit_status(0),
            stdout: b"not-a-sha file\n".to_vec(),
            stderr: Vec::new(),
        };

        assert!(parse_sha256_output(Path::new("object.o"), output).is_err());
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code)
    }

    #[cfg(windows)]
    fn exit_status(code: u32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code)
    }
}
