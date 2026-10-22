//! Live observation is independent of mutation support. Missing prerequisites
//! produce an explicit non-success receipt, never a successful skip.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use clap::Subcommand;
use serde_json::{Value, json};
mod affinity;
mod evidence;
mod game_runnable;
mod kernel_compat;
pub(crate) mod recovery;

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum Suite {
    KernelCompat {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
    },
    GameRunnable {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(10..=60))]
        seconds: u64,
    },
    SuspendResume {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
        /// Required: this suite really suspends the machine.
        #[arg(long)]
        allow_system_suspend: bool,
        #[arg(long, default_value = "systemctl suspend")]
        suspend_command: String,
        /// Arm an RTC wake alarm with `rtcwake -m no` first (needs permission).
        #[arg(long)]
        rtcwake_seconds: Option<u64>,
        #[arg(long, default_value_t = 600)]
        resume_timeout_seconds: u64,
    },
    Recovery {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
    },
    Affinity {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
    },
    Observation {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        stutter_bin: PathBuf,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(3..=60))]
        seconds: u64,
    },
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(
    program: &Path,
    args: &[String],
    out: &Path,
    name: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let stdout = fs::File::create(out.join(format!("{name}.stdout")))?;
    let stderr = fs::File::create(out.join(format!("{name}.stderr")))?;
    let mut child = OwnedChild(
        Command::new(program)
            .args(args)
            .env("STUTTER_CONFIG", out.join("acceptance.toml"))
            .env("RUST_LOG", "info")
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .with_context(|| format!("start {name}"))?,
    );
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "{name} exited {status}; inspect retained logs"
            );
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "{name} exceeded its deadline");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}
fn start_ticks(pid: u32) -> anyhow::Result<u64> {
    fs::read_to_string(format!("/proc/{pid}/stat"))?
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|value| value.parse().ok())
        .context("process start-time identity unavailable")
}
fn allowed_cpu() -> anyhow::Result<u32> {
    let status = fs::read_to_string("/proc/self/status")?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .and_then(|value| value.trim().split([',', '-']).next())
        .and_then(|value| value.parse().ok())
        .context("no allowed CPU for disposable workload")
}

pub fn workload(seconds: u64, ready: &Path, descendants: bool) -> anyhow::Result<()> {
    ensure!(
        (3..=90).contains(&seconds),
        "workload duration outside bound"
    );
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut children = Vec::new();
    if descendants {
        for index in 0..2 {
            let child_ready = ready.with_extension(format!("child-{index}"));
            let mut child = OwnedChild(
                Command::new("sh")
                    .args([
                        "-c",
                        "printf ready > \"$1\"; read -r stop",
                        "acceptance-child",
                    ])
                    .arg(&child_ready)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?,
            );
            let ready_deadline = Instant::now() + Duration::from_secs(5);
            while !child_ready.exists() {
                ensure!(
                    child.0.try_wait()?.is_none(),
                    "descendant exited before readiness"
                );
                ensure!(
                    Instant::now() < ready_deadline,
                    "descendant readiness timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            // The parent's owned stdin writer remains open. Parent death closes
            // it in the kernel, so the child's read receives EOF and exits.
            children.push(child);
        }
    }
    let busy = std::thread::Builder::new()
        .name("accept-busy".into())
        .spawn(move || {
            let mut value = 1_u64;
            while Instant::now() < deadline {
                value = std::hint::black_box(value.wrapping_mul(1664525).wrapping_add(1013904223));
            }
        })?;
    let wake = std::thread::Builder::new()
        .name("accept-wake".into())
        .spawn(move || {
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
                std::hint::black_box(Instant::now());
            }
        })?;
    let temporary = ready.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    serde_json::to_writer(
        &mut file,
        &json!({"pid":std::process::id(), "starttime_ticks":start_ticks(std::process::id())?}),
    )?;
    file.sync_all()?;
    fs::rename(temporary, ready)?;
    busy.join()
        .map_err(|_| anyhow::anyhow!("busy workload panicked"))?;
    wake.join()
        .map_err(|_| anyhow::anyhow!("wakeup workload panicked"))?;
    Ok(())
}

pub fn run(root: &Path, suite: Suite) -> anyhow::Result<()> {
    let (out, stutter_bin, seconds) = match suite {
        Suite::Observation {
            out,
            stutter_bin,
            seconds,
        } => (out, stutter_bin, seconds),
        Suite::Affinity { out, stutter_bin } => return affinity::run(root, &out, &stutter_bin),
        Suite::KernelCompat { out, stutter_bin } => {
            return kernel_compat::run(root, &out, &stutter_bin);
        }
        Suite::GameRunnable {
            out,
            stutter_bin,
            seconds,
        } => {
            return game_runnable::run(root, &out, &stutter_bin, seconds);
        }
        Suite::Recovery { out, stutter_bin } => return recovery::run(root, &out, &stutter_bin),
        Suite::SuspendResume {
            out,
            stutter_bin,
            allow_system_suspend,
            suspend_command,
            rtcwake_seconds,
            resume_timeout_seconds,
        } => {
            return recovery::run_suspend(
                root,
                &out,
                &stutter_bin,
                &recovery::SuspendOptions {
                    allow_system_suspend,
                    suspend_command,
                    rtcwake_seconds,
                    resume_timeout_seconds,
                },
            );
        }
    };
    ensure!(cfg!(target_os = "linux"), "live acceptance requires Linux");
    fs::create_dir(&out)
        .context("acceptance output must be a new directory with an existing parent")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(stutter_bin);
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated observation defaults; no user configuration.\n",
    )?;
    let mut stage = "preflight";
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(
            root,
            &out,
            &binary,
            evidence::LiveAcceptanceSuite::Observation,
        )?;
        command(
            &binary,
            &args(&["doctor", "--json"]),
            &out,
            "doctor",
            Duration::from_secs(15),
        )?;
        let doctor: Value = serde_json::from_slice(&fs::read(out.join("doctor.stdout"))?)?;
        ensure!(
            doctor["overall"] == "Pass" || doctor["overall"] == "Warn",
            "doctor preflight failed"
        );
        command(
            &binary,
            &args(&["doctor", "tracepoints", "--dump"]),
            &out,
            "tracepoints",
            Duration::from_secs(15),
        )?;
        let cpu = allowed_cpu()?;
        let ready = out.join("workload-ready.json");
        let mut child = OwnedChild(
            Command::new("taskset")
                .args(["-c", &cpu.to_string()])
                .arg(std::env::current_exe()?)
                .args([
                    "live-workload",
                    "--seconds",
                    &(seconds + 20).to_string(),
                    "--ready",
                ])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(fs::File::create(out.join("workload.stderr"))?)
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            ensure!(
                child.0.try_wait()?.is_none(),
                "workload exited before readiness"
            );
            ensure!(Instant::now() < deadline, "workload readiness timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid = child.0.id();
        let ticks = start_ticks(pid)?;
        stage = "record";
        let recordings = out.join("recording");
        let mut record_args = args(&[
            "record",
            "--tree-pid",
            &pid.to_string(),
            "--duration",
            &seconds.to_string(),
            "--summary-ms",
            "250",
            "--no-hwmon",
            "--no-cpu-freq",
            "--no-faults",
            "--no-block-io",
            "--no-stat-wait",
            "--out-dir",
        ]);
        record_args.push(recordings.to_string_lossy().into_owned());
        command(
            &binary,
            &record_args,
            &out,
            "record",
            Duration::from_secs(seconds + 15),
        )?;
        ensure!(
            child.0.try_wait()?.is_none() && start_ticks(pid)? == ticks,
            "workload identity did not survive recording"
        );
        drop(child);
        stage = "artifacts";
        let recording = evidence::recording_dir(&recordings)?;
        let measurements = evidence::validate(&recording, pid, ticks)?;
        let run = recording.to_string_lossy().into_owned();
        command(
            &binary,
            &args(&["validate", &run, "--json"]),
            &out,
            "validate",
            Duration::from_secs(20),
        )?;
        let validation: Value = serde_json::from_slice(&fs::read(out.join("validate.stdout"))?)?;
        ensure!(
            validation["passed"] == true,
            "artifact validator did not report success"
        );
        stage = "report";
        command(
            &binary,
            &args(&["report", &run, "--analysis-json"]),
            &out,
            "analysis",
            Duration::from_secs(20),
        )?;
        let analysis: Value = serde_json::from_slice(&fs::read(out.join("analysis.stdout"))?)?;
        ensure!(
            analysis.get("cluster_analysis").is_some(),
            "diagnosis analysis missing"
        );
        let html = out.join("report.html");
        command(
            &binary,
            &args(&["report", &run, "--html", &html.to_string_lossy()]),
            &out,
            "html",
            Duration::from_secs(20),
        )?;
        ensure!(fs::metadata(html)?.len() > 0, "empty HTML report");
        command(
            &binary,
            &args(&["advisor", "--run", &run, "--json"]),
            &out,
            "advisor",
            Duration::from_secs(20),
        )?;
        let _: Value = serde_json::from_slice(&fs::read(out.join("advisor.stdout"))?)?;
        Ok(measurements)
    })();
    let status = if result.is_ok() {
        "PASS"
    } else if stage == "preflight" {
        "BLOCKED"
    } else {
        "FAIL"
    };
    let receipt = json!({"schema_version":1,"status":status,"stage":stage,
        "evidence_kind":"controlled_real","measurements":result.as_ref().ok(),
        "error":result.as_ref().err().map(|error| format!("{error:#}")),
        "mutation_suite":"not_run","privacy":"local raw logs; inspect and sanitize before publication",
        "scope":"real scheduler observation of an owned generic workload through record, validation and report",
        "not_claimed":["GameRunnableLatency objective (generic workload only)","kernel compatibility beyond this host","profiler overhead","portable verified bundle until exported"]});
    evidence::write_result(&out, evidence::LiveAcceptanceSuite::Observation, receipt)?;
    println!("{status}: evidence {}", out.display());
    result.map(|_| ())
}
