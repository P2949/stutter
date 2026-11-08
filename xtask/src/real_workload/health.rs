//! GPU health during a run: any amdgpu reset, timeout or hang in the kernel log aborts the study.
use std::process::Command;

/// Kernel log lines since `since_unix` that indicate a GPU fault, or `None` when the kernel log
/// cannot be read (reported as unobserved, never as healthy).
pub fn gpu_faults_since(since_unix: u64) -> Option<Vec<String>> {
    let output = Command::new("journalctl")
        .args(["-k", "--no-pager", "-o", "cat", "--since"])
        .arg(format!("@{since_unix}"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| fault_lines(&String::from_utf8_lossy(&output.stdout)))
}

pub fn fault_lines(log: &str) -> Vec<String> {
    log.lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            (lower.contains("amdgpu") || lower.contains("[drm"))
                && [
                    "gpu reset",
                    "ring gfx",
                    "ring sdma",
                    "ring comp",
                    "ring timeout",
                    "gpu hang",
                    "vram is lost",
                    "gpu fault",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
        })
        .map(str::to_owned)
        .take(20)
        .collect()
}
