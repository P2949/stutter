//! The representative real-workload mode of the overhead suite (#38). The A..F probe-set
//! comparison, counterbalanced order and sampling are exactly the disposable workload's; only
//! the workload differs. The operator supplies a repeatable fixed-work command (a benchmark
//! scene, a fixed build unit, ...), so the workload, its settings and the scene are identical
//! in every run by construction. Local identity never enters the evidence: the receipt keeps a
//! label, the program's file name, the argument count and digests, not paths or arguments.
use super::*;
use crate::evidence_bundle::sha256::sha256_hex;

/// Margin by which the workload must outlive the profiler window, so every probe-set run
/// profiles the same part of a still-running workload.
pub(super) const MIN_MARGIN_SECONDS: f64 = 2.0;

#[derive(Clone, Debug)]
pub(super) struct ExternalWorkload {
    pub label: String,
    pub command: Vec<String>,
    /// Fixed work units one run performs (1 = runs per second).
    pub work_units: f64,
    /// Longest a measured run may take; set from the unmeasured warm-up run.
    pub limit: Duration,
}

impl ExternalWorkload {
    pub fn new(label: &str, command: &[String], work_units: Option<f64>) -> anyhow::Result<Self> {
        let label = label.trim();
        ensure!(
            !label.is_empty()
                && label.len() <= 64
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "--workload-label must be 1-64 characters of [A-Za-z0-9._-]"
        );
        let program = command
            .first()
            .context("a workload command is required after `--`")?;
        ensure!(
            Path::new(program).file_name().is_some() && !program.trim().is_empty(),
            "workload program is empty"
        );
        let work_units = work_units.unwrap_or(1.0);
        ensure!(
            work_units.is_finite() && work_units > 0.0,
            "--work-units must be positive"
        );
        Ok(Self {
            label: label.to_owned(),
            command: command.to_vec(),
            work_units,
            limit: Duration::from_secs(600),
        })
    }

    /// Throughput of one completed fixed-work run.
    pub fn throughput(&self, elapsed_seconds: f64) -> f64 {
        self.work_units / elapsed_seconds
    }

    /// The workload's own output is never retained: arbitrary application output cannot be
    /// sanitized generically, so it goes nowhere and is not exportable evidence.
    pub fn spawn(&self) -> anyhow::Result<OwnedChild> {
        Ok(OwnedChild(
            Command::new(&self.command[0])
                .args(&self.command[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .unprivileged()
                .spawn()
                .with_context(|| format!("start workload {}", self.label))?,
        ))
    }

    /// Waits for a run to finish within `limit`, failing closed on a non-zero exit.
    pub fn wait(&self, child: &mut OwnedChild) -> anyhow::Result<()> {
        let deadline = Instant::now() + self.limit;
        loop {
            if let Some(status) = child.0.try_wait()? {
                ensure!(status.success(), "workload exited with {status}");
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "workload exceeded its {} s limit",
                self.limit.as_secs()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// One unmeasured run before the rounds: it warms the page cache identically for every
    /// configuration's first run, proves the workload terminates successfully and outlives the
    /// profiler window, and bounds how long a measured run may take. Returns elapsed seconds.
    pub fn warm_up(&mut self, dir: &Path, window_seconds: u64) -> anyhow::Result<f64> {
        fs::create_dir_all(dir)?;
        let started = Instant::now();
        let mut child = self.spawn()?;
        self.wait(&mut child)?;
        let elapsed = started.elapsed().as_secs_f64();
        ensure!(
            elapsed >= window_seconds as f64 + MIN_MARGIN_SECONDS,
            "workload ran {elapsed:.1} s but must outlive the {window_seconds} s profiling window by \
             {MIN_MARGIN_SECONDS} s; use a longer fixed-work scene"
        );
        self.limit = Duration::from_secs_f64(elapsed * 3.0 + 30.0);
        Ok(elapsed)
    }

    /// What the receipt records about the workload: no paths, no argument text.
    pub fn identity(&self) -> anyhow::Result<Value> {
        let program = Path::new(&self.command[0]);
        let resolved = if self.command[0].contains('/') {
            Some(PathBuf::from(&self.command[0]))
        } else {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join(program))
                    .find(|candidate| candidate.is_file())
            })
        };
        let program_sha256 = resolved
            .map(|path| fs::read(path).map(|bytes| sha256_hex(&bytes)))
            .transpose()?;
        Ok(json!({
            "label": self.label,
            "program": program.file_name().map(|name| name.to_string_lossy().into_owned()),
            "argument_count": self.command.len() - 1,
            "command_sha256": sha256_hex(self.command.join("\u{1f}").as_bytes()),
            "program_sha256": program_sha256,
            "work_units_per_run": self.work_units,
            "declared_by": "operator: the harness does not verify that the workload is representative",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{overhead, *};

    fn command(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn labels_commands_and_work_units_are_validated() {
        let ok = ExternalWorkload::new("scene-1", &command(&["true"]), Some(2.0)).unwrap();
        assert_eq!(ok.throughput(4.0), 0.5);
        for bad in ["", "  ", "has space", "slash/label", "é"] {
            assert!(
                ExternalWorkload::new(bad, &command(&["true"]), None).is_err(),
                "{bad:?}"
            );
        }
        assert!(ExternalWorkload::new("x", &[], None).is_err());
        for units in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(ExternalWorkload::new("x", &command(&["true"]), Some(units)).is_err());
        }
    }

    #[test]
    fn identity_keeps_no_paths_or_argument_text() {
        let workload = ExternalWorkload::new(
            "bench",
            &command(&["sh", "-c", "echo /home/someone/secret"]),
            None,
        )
        .unwrap();
        let identity = workload.identity().unwrap();
        let text = identity.to_string();
        assert!(
            !text.contains("someone") && !text.contains("/home"),
            "{text}"
        );
        assert_eq!(identity["program"], "sh");
        assert_eq!(identity["argument_count"], 2);
        assert_eq!(identity["command_sha256"].as_str().unwrap().len(), 64);
        // A different argument list is a different workload identity.
        let other = ExternalWorkload::new("bench", &command(&["sh", "-c", "echo x"]), None)
            .unwrap()
            .identity()
            .unwrap();
        assert_ne!(identity["command_sha256"], other["command_sha256"]);
    }

    #[test]
    fn warm_up_requires_success_and_outliving_the_profiler_window() {
        let dir = std::env::temp_dir().join(format!("stutter-external-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut failing = ExternalWorkload::new("f", &command(&["false"]), None).unwrap();
        assert!(failing.warm_up(&dir, 0).is_err());
        let mut short = ExternalWorkload::new("s", &command(&["true"]), None).unwrap();
        let error = short.warm_up(&dir, 10).unwrap_err().to_string();
        assert!(error.contains("outlive"), "{error}");
        let mut long = ExternalWorkload::new("l", &command(&["sleep", "2"]), None).unwrap();
        let elapsed = long.warm_up(&dir, 0).unwrap();
        assert!(elapsed >= 2.0);
        assert!(long.limit.as_secs_f64() > elapsed * 3.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unprofiled_run_measures_fixed_work_throughput_of_the_real_command() {
        let dir = std::env::temp_dir().join(format!("stutter-external-run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let workload =
            ExternalWorkload::new("sleepy", &command(&["sleep", "1"]), Some(4.0)).unwrap();
        // Configuration A runs no profiler, so the binary and window are unused.
        let sample = overhead::run_one(
            Path::new("/nonexistent/stutter"),
            overhead::Workload::External(&workload),
            &dir,
            None,
            10,
        )
        .unwrap();
        // 4 work units in about one second.
        assert!(
            sample.throughput > 2.0 && sample.throughput <= 4.0,
            "{}",
            sample.throughput
        );
        assert!(sample.profiler_cpu_percent.is_none() && sample.event_rate.is_none());
        assert!(!dir.join("workload.stdout").exists() && !dir.join("workload.stderr").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stutter-ext-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in profiler: exits successfully after `seconds` without recording anything.
    fn fake_profiler(dir: &Path, seconds: u64) -> PathBuf {
        let path = dir.join("fake-stutter");
        fs::write(&path, format!("#!/bin/sh\nsleep {seconds}\n")).unwrap();
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        path
    }

    fn flags() -> Vec<String> {
        vec!["--no-hwmon".to_owned()]
    }

    #[test]
    fn liveness_check_rejects_any_early_exit_and_accepts_a_running_workload() {
        let running = ExternalWorkload::new("r", &command(&["sleep", "5"]), None).unwrap();
        let mut child = running.spawn().unwrap();
        overhead::ensure_workload_alive(&mut child).unwrap();
        drop(child);
        for early in [command(&["true"]), command(&["false"])] {
            let workload = ExternalWorkload::new("e", &early, None).unwrap();
            let mut child = workload.spawn().unwrap();
            while child.0.try_wait().unwrap().is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            // Successful and nonzero early exits both invalidate the run.
            let error = overhead::ensure_workload_alive(&mut child)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("before the profiling window ended"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_measured_run_that_exits_inside_the_window_is_rejected_even_after_a_good_warm_up() {
        let dir = scratch("early");
        let mut workload = ExternalWorkload::new("w", &command(&["sleep", "3"]), None).unwrap();
        workload.warm_up(&dir.join("warm-up"), 0).unwrap();
        // The measured invocation now exits at once, while the profiler is still running.
        let early = ExternalWorkload::new("w", &command(&["true"]), None).unwrap();
        let error = overhead::run_one(
            &fake_profiler(&dir, 4),
            overhead::Workload::External(&early),
            &dir.join("run"),
            Some(&flags()),
            4,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("before the profiling window ended"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_workload_that_outlives_the_window_passes_the_liveness_checks() {
        let dir = scratch("alive");
        let alive = ExternalWorkload::new("w", &command(&["sleep", "5"]), None).unwrap();
        // The fake profiler records nothing, so the run later fails on the missing
        // recording, which proves the liveness checks themselves were satisfied.
        let error = overhead::run_one(
            &fake_profiler(&dir, 1),
            overhead::Workload::External(&alive),
            &dir.join("run"),
            Some(&flags()),
            1,
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("profiling window"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_nonzero_workload_exit_after_the_window_rejects_the_run() {
        let dir = scratch("nonzero");
        let failing =
            ExternalWorkload::new("w", &command(&["sh", "-c", "sleep 2; exit 3"]), None).unwrap();
        let error = overhead::run_one(
            Path::new("/nonexistent/stutter"),
            overhead::Workload::External(&failing),
            &dir.join("run"),
            None,
            1,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("workload exited"), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn raw_evidence_keeps_neither_workload_output_nor_argument_text() {
        let dir = scratch("privacy");
        let marker = "PRIVATE-ARGUMENT-MARKER";
        let script = format!("echo {marker}; echo {marker} >&2; sleep 1");
        let workload = ExternalWorkload::new("w", &command(&["sh", "-c", &script]), None).unwrap();
        overhead::run_one(
            Path::new("/nonexistent/stutter"),
            overhead::Workload::External(&workload),
            &dir.join("run"),
            None,
            1,
        )
        .unwrap();
        let identity = workload.identity().unwrap().to_string();
        assert!(!identity.contains(marker), "{identity}");
        for entry in fs::read_dir(dir.join("run")).unwrap() {
            let path = entry.unwrap().path();
            assert!(
                !fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains(marker),
                "{}",
                path.display()
            );
            assert!(
                !path.ends_with("workload.stdout") && !path.ends_with("workload.stderr"),
                "{}",
                path.display()
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_workloads_run_with_no_capabilities_and_no_new_privileges() {
        let dir = scratch("privileges");
        let report = dir.join("status");
        let script = format!(
            "grep -E '^(CapInh|CapPrm|CapEff|CapAmb|NoNewPrivs):' /proc/self/status > {}",
            report.display()
        );
        let workload = ExternalWorkload::new("w", &command(&["sh", "-c", &script]), None).unwrap();
        let mut child = workload.spawn().unwrap();
        workload.wait(&mut child).unwrap();
        let status = fs::read_to_string(&report).unwrap();
        for name in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
            let line = status.lines().find(|l| l.starts_with(name)).unwrap();
            assert!(line.ends_with("0000000000000000"), "{line}");
        }
        assert!(status.contains("NoNewPrivs:\t1"), "{status}");
        let _ = fs::remove_dir_all(&dir);
    }
}
