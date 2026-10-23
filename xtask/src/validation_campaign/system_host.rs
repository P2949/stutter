//! The real [`CaptureHost`]: drives `vkmark`/`fio`/UDP traffic as the campaign user and asks the
//! privileged runner (typed requests only) for the scheduler burst and the recording. Every
//! intervention is verified from independent telemetry (`/proc`, `fio` JSON, `/proc/interrupts`,
//! DRM busy percent), never from Stutter's own diagnosis output.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use serde_json::Value;
use stutter_validation_runner::{
    client::Client,
    model::{RecordProbes, Request, ResponseStatus},
};

use super::{
    scenarios::{
        capture::InterventionReport,
        exec::{CaptureHost, RecordSummary, TargetIdentity},
        fixture::HostFacts,
        model::{InterventionKind, ScenarioSet, ScenarioSpec, Target},
        probe::{self, gpu_busy_established, scheduler_burst_established},
    },
    workloads::{
        CommandSpec, SystemTools,
        fio::{self, FioProfile},
        network_irq::{self, NicSource},
        vkmark::{self, VkmarkConfig},
    },
};

pub const BURST_MS: u64 = 4;
pub const BURST_PERIOD_MS: u64 = 20;
pub const BURST_PRIORITY: u32 = 50;
const MIN_IRQ_DELTA: u64 = 200;
const MIN_GPU_BUSY_PCT: u32 = 90;
const MIN_BLOCKER_P99_NS: u64 = 5_000_000;
const TRAFFIC_PPS: u32 = 4_000;
const STEER_ATTEMPTS: u32 = 40;
/// Interventions outlast the recording by this much so they can still be inspected after it.
const TAIL_SECONDS: u64 = 15;

/// Everything the host needs; derived from campaign state, never from a prompt.
pub struct HostSettings {
    pub socket: PathBuf,
    pub campaign_id: String,
    pub manifest_sha256: String,
    pub task: &'static str,
    pub task_attempt: u32,
    pub capture: u32,
    pub scratch: PathBuf,
    pub vkmark: super::scenarios::model::VkmarkParams,
    pub nic: Option<NicSource>,
    pub irq: Option<u32>,
    pub workload_version: String,
    pub burst_ms: u64,
    pub burst_period_ms: u64,
}

pub struct SystemCaptureHost {
    settings: HostSettings,
    children: Vec<Child>,
    capture_dir: PathBuf,
    contender: Option<u32>,
    traffic: Option<JoinHandle<()>>,
    interrupts_before: Option<std::collections::BTreeMap<u32, u64>>,
    gpu_samples: Vec<u32>,
    started: Option<std::time::Instant>,
}

impl SystemCaptureHost {
    pub fn new(settings: HostSettings) -> Self {
        Self {
            settings,
            children: Vec::new(),
            capture_dir: PathBuf::new(),
            contender: None,
            traffic: None,
            interrupts_before: None,
            gpu_samples: Vec::new(),
            started: None,
        }
    }

    fn client(&self) -> Client<'_> {
        Client {
            socket: &self.settings.socket,
            campaign_id: &self.settings.campaign_id,
            manifest_sha256: &self.settings.manifest_sha256,
        }
    }

    fn send(&self, request: Request) -> Result<Value, String> {
        let response = self
            .client()
            .send(self.settings.task, request)
            .map_err(|error| format!("the validation runner is unreachable: {error}"))?;
        match response.status {
            ResponseStatus::Ok => Ok(response.data.unwrap_or(Value::Null)),
            _ => {
                let error = response
                    .error
                    .map(|e| format!("{:?}: {}", e.code, e.message));
                Err(format!("runner refused: {}", error.unwrap_or_default()))
            }
        }
    }

    fn spawn(
        &mut self,
        spec: &CommandSpec,
        log: &Path,
        stdout_to_log: bool,
    ) -> Result<u32, String> {
        let file = fs::File::create(log).map_err(|e| e.to_string())?;
        let mut command = Command::new(&spec.program);
        command.args(&spec.args).envs(spec.env.iter().cloned());
        command.stdin(Stdio::null());
        if stdout_to_log {
            command.stdout(file.try_clone().map_err(|e| e.to_string())?);
        } else {
            command.stdout(Stdio::null());
        }
        let child = command
            .stderr(file)
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", spec.program))?;
        let pid = child.id();
        self.children.push(child);
        Ok(pid)
    }

    fn window(&self, spec: &ScenarioSpec) -> u64 {
        spec.seconds + TAIL_SECONDS + 10
    }

    fn vkmark_config(&self, target: Target) -> VkmarkConfig {
        match target {
            Target::VkmarkGpuHeavy => VkmarkConfig::heavy(&self.settings.vkmark),
            _ => VkmarkConfig::light(&self.settings.vkmark),
        }
    }
}

/// Start time (clock ticks since boot) from `/proc/<pid>/stat`, tolerant of odd command names.
pub fn starttime_ticks(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// The CPU a process last ran on (`processor`, field 39).
pub fn last_cpu(stat: &str) -> Option<u32> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(36)?
        .parse()
        .ok()
}

fn read_proc(pid: u32, file: &str) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/{file}")).ok()
}

/// A second render window needs a few seconds to reach full load; recording (and the busy
/// sampler) start only once the GPU has stayed saturated for two samples, or after 20 s.
fn wait_for_gpu_saturation() {
    let mut consecutive = 0;
    for _ in 0..20 {
        consecutive = if gpu_busy_percent().is_some_and(|busy| busy >= MIN_GPU_BUSY_PCT) {
            consecutive + 1
        } else {
            0
        };
        if consecutive >= 2 {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn gpu_busy_percent() -> Option<u32> {
    fs::read_dir("/sys/class/drm")
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            fs::read_to_string(entry.path().join("device/gpu_busy_percent"))
                .ok()?
                .trim()
                .parse()
                .ok()
        })
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

impl CaptureHost for SystemCaptureHost {
    fn set_capture_dir(&mut self, dir: &Path) {
        self.capture_dir = dir.to_path_buf();
        if let Some(n) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse().ok())
        {
            self.settings.capture = n;
        }
        // `.../attempts/<task>/<attempt>/captures/<capture>`
        if let Some(n) = dir
            .parent()
            .and_then(Path::parent)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse().ok())
        {
            self.settings.task_attempt = n;
        }
    }

    fn preflight(&mut self, spec: &ScenarioSpec) -> Result<(), String> {
        let tools = SystemTools;
        let mut wanted: Vec<&str> = Vec::new();
        let vkmark_target = matches!(spec.target, Target::VkmarkLight | Target::VkmarkGpuHeavy);
        if vkmark_target || spec.interventions.contains(&InterventionKind::GpuStressor) {
            vkmark::preflight(
                &tools,
                &[
                    &self.settings.vkmark.light_scene,
                    &self.settings.vkmark.heavy_scene,
                ],
            )?;
        }
        if spec.target == Target::FioFixedWork
            || spec.interventions.contains(&InterventionKind::BlockingIo)
        {
            wanted.push("fio");
        }
        if !wanted.is_empty() {
            super::workloads::preflight_tools(&tools, &wanted).map_err(|p| p.join("; "))?;
        }
        if spec
            .interventions
            .contains(&InterventionKind::NetworkTraffic)
            && self.settings.nic.is_none()
        {
            return Err("no independently controllable NIC IRQ source was discovered".to_owned());
        }
        fs::create_dir_all(self.capture_dir.join("mangohud")).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.settings.scratch).map_err(|e| e.to_string())
    }

    fn start_target(&mut self, spec: &ScenarioSpec) -> Result<TargetIdentity, String> {
        let seconds = self.window(spec);
        let command = match spec.target {
            Target::FioFixedWork => {
                fio::target_command(&FioProfile::target(), &self.settings.scratch, seconds)
            }
            other => vkmark::command(
                &self.vkmark_config(other),
                seconds,
                &self.capture_dir.join("mangohud"),
            ),
        };
        let log = self.capture_dir.join("target.log");
        let pid = self.spawn(&command, &log, spec.target == Target::FioFixedWork)?;
        std::thread::sleep(Duration::from_secs(3));
        let ticks = read_proc(pid, "stat")
            .and_then(|stat| starttime_ticks(&stat))
            .ok_or("the target exited before it could be identified")?;
        self.started = Some(std::time::Instant::now());
        Ok(TargetIdentity {
            pid,
            starttime_ticks: ticks,
        })
    }

    fn start_interventions(
        &mut self,
        spec: &ScenarioSpec,
        target: TargetIdentity,
    ) -> Result<(), String> {
        let seconds = self.window(spec);
        for kind in &spec.interventions {
            match kind {
                InterventionKind::SchedulerBurst => {
                    let cpu = read_proc(target.pid, "stat")
                        .and_then(|stat| last_cpu(&stat))
                        .ok_or("cannot read the target's CPU")?;
                    let data = self.send(Request::ExecuteSchedulerBurst {
                        cpu,
                        burst_ms: self.settings.burst_ms,
                        period_ms: self.settings.burst_period_ms,
                        seconds,
                    })?;
                    self.contender = data["pid"].as_u64().map(|p| p as u32);
                }
                InterventionKind::BlockingIo => {
                    let command = fio::blocker_command(
                        &FioProfile::blocker(),
                        &self.settings.scratch,
                        seconds,
                    );
                    self.spawn(&command, &self.capture_dir.join("blocker.json"), true)?;
                }
                InterventionKind::GpuStressor => {
                    // Bare vkmark: per-frame MangoHud logging slows frame submission enough to
                    // leave the GPU short of saturation (94% against 99% measured here).
                    let command =
                        vkmark::bare_command(&VkmarkConfig::heavy(&self.settings.vkmark), seconds);
                    self.spawn(&command, &self.capture_dir.join("stressor.log"), false)?;
                }
                InterventionKind::NetworkTraffic => {
                    let nic = self.settings.nic.clone().ok_or("no NIC source")?;
                    let irq = self.settings.irq.ok_or("no IRQ was frozen")?;
                    let target_addr = std::net::SocketAddr::from((nic.gateway, 9));
                    // The flow's queue is random per socket: keep only one that drives the
                    // frozen IRQ, then measure the capture against a baseline taken after it.
                    let socket = network_irq::steer_flow(
                        target_addr,
                        irq,
                        STEER_ATTEMPTS,
                        super::host_factory::flow_plan(),
                        &super::host_factory::read_interrupts,
                    )?;
                    self.interrupts_before = Some(super::host_factory::read_interrupts()?);
                    self.traffic = Some(std::thread::spawn(move || {
                        network_irq::pump(&socket, TRAFFIC_PPS, 512, Duration::from_secs(seconds));
                    }));
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
        if spec.interventions.contains(&InterventionKind::GpuStressor) {
            wait_for_gpu_saturation();
        }
        Ok(())
    }

    fn record(
        &mut self,
        spec: &ScenarioSpec,
        target: TargetIdentity,
        raw: &Path,
    ) -> Result<RecordSummary, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let sampler = spec
            .interventions
            .contains(&InterventionKind::GpuStressor)
            .then(|| {
                let flag = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut samples = Vec::new();
                    while !flag.load(Ordering::Relaxed) {
                        samples.extend(gpu_busy_percent());
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    samples
                })
            });
        let probes = RecordProbes::Corpus {
            irq: self.settings.irq,
        };
        let response = self.send(Request::ExecuteRecording {
            attempt: self.settings.task_attempt,
            capture: self.settings.capture,
            target_pid: target.pid,
            target_starttime_ticks: target.starttime_ticks,
            seconds: spec.seconds,
            probes,
        });
        stop.store(true, Ordering::Relaxed);
        self.gpu_samples = sampler
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        let data = response?;
        if data["exit_code"].as_i64() != Some(0) {
            // The runner keeps the recorder's output next to the attempt; its tail says why.
            let log = self
                .capture_dir
                .parent()
                .and_then(Path::parent)
                .map(|attempt| attempt.join("logs/record.log"))
                .and_then(|path| fs::read_to_string(path).ok())
                .map(|text| {
                    let tail: Vec<&str> = text.lines().rev().take(3).collect();
                    tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
                })
                .unwrap_or_default();
            return Err(format!(
                "stutter record exited {}: {log}",
                data["exit_code"]
            ));
        }
        summarize_recording(raw)
    }

    fn verify_interventions(
        &mut self,
        spec: &ScenarioSpec,
        _raw: &Path,
    ) -> Vec<InterventionReport> {
        let elapsed = self.started.map_or(0, |s| s.elapsed().as_secs());
        spec.interventions
            .iter()
            .map(|kind| {
                let result = match kind {
                    InterventionKind::SchedulerBurst => self
                        .contender
                        .and_then(|pid| read_proc(pid, "stat"))
                        .ok_or_else(|| "the contender is gone".to_owned())
                        .and_then(|stat| {
                            scheduler_burst_established(
                                &stat,
                                BURST_PRIORITY,
                                self.settings.burst_ms,
                                self.settings.burst_period_ms,
                                elapsed,
                            )
                        }),
                    InterventionKind::BlockingIo => self.verify_blocker(),
                    InterventionKind::GpuStressor => gpu_busy_established(
                        &self.gpu_samples,
                        MIN_GPU_BUSY_PCT,
                        (spec.seconds / 2) as usize,
                    ),
                    InterventionKind::NetworkTraffic => self.verify_irq(),
                };
                InterventionReport {
                    kind: *kind,
                    verified: result.is_ok(),
                    detail: result.err().unwrap_or_default(),
                }
            })
            .collect()
    }

    fn stop_all(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.children.clear();
        if let Some(handle) = self.traffic.take() {
            let _ = handle.join();
        }
        self.contender = None;
        self.interrupts_before = None;
    }

    fn sanitize(&mut self, raw: &Path, out: &Path, name: &str) -> Result<(), String> {
        let mut command = Command::new("python3");
        command
            .args(["scripts/sanitize-run-artifact.py", "--input"])
            .arg(raw)
            .arg("--output")
            .arg(out)
            // The sanitizer's own `cargo run` verification would rebuild (and re-link) the pinned
            // binary; the campaign validates with the pinned binary itself.
            .args(["--name", name, "--no-verify"]);
        run_ok(&mut command, "the sanitizer")
    }

    fn data_quality(&mut self, dir: &Path) -> Result<String, String> {
        let output = Command::new("target/debug/stutter")
            .args(["report"])
            .arg(dir)
            .arg("--analysis-json")
            .output()
            .map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        find_key(&value, "data_quality_level")
            .or_else(|| find_key(&value, "data_quality").and_then(|q| q.get("level")))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "the analysis reports no data quality".to_owned())
    }

    fn validate(&mut self, dir: &Path) -> Result<(), String> {
        run_ok(
            Command::new("target/debug/stutter")
                .arg("validate")
                .arg(dir),
            "stutter validate",
        )
    }

    fn facts(&self) -> (HostFacts, String) {
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        let (cpu_vendor, cpu_topology_bucket) = probe::cpu_bucket(&read("/proc/cpuinfo"));
        let gpu_vendor = fs::read_dir("/sys/class/drm")
            .ok()
            .and_then(|d| {
                d.filter_map(Result::ok)
                    .find_map(|e| fs::read_to_string(e.path().join("device/vendor")).ok())
            })
            .map_or("Unknown", |id| probe::gpu_vendor(&id));
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_owned());
        (
            HostFacts {
                kernel: read("/proc/sys/kernel/osrelease").trim().to_owned(),
                distribution: probe::os_release_summary(&read("/etc/os-release")),
                cpu_vendor,
                cpu_topology_bucket,
                gpu_vendor: gpu_vendor.to_owned(),
                gpu_driver: "kernel-default".to_owned(),
                compositor: "session-default".to_owned(),
                session_type: session,
                workload_version: self.settings.workload_version.clone(),
            },
            "corpus".to_owned(),
        )
    }
}

impl SystemCaptureHost {
    fn verify_blocker(&self) -> Result<(), String> {
        let text = fs::read_to_string(self.capture_dir.join("blocker.json")).unwrap_or_default();
        // fio prints its JSON only when it ends; a blocker still running was clearly active.
        if text.trim().is_empty() {
            return if self.children.iter().any(|_| true) {
                Ok(())
            } else {
                Err("the blocker never started".to_owned())
            };
        }
        fio::blocking_established(&fio::summarize(&text)?, 50, MIN_BLOCKER_P99_NS)
    }

    fn verify_irq(&self) -> Result<(), String> {
        let irq = self.settings.irq.ok_or("no IRQ was frozen")?;
        let before = self
            .interrupts_before
            .as_ref()
            .ok_or("no interrupt baseline")?;
        let after = network_irq::parse_interrupts(
            &fs::read_to_string("/proc/interrupts").map_err(|e| e.to_string())?,
        );
        network_irq::counter_rose(before, &after, irq, MIN_IRQ_DELTA).map(|_| ())
    }
}

fn run_ok(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command.output().map_err(|e| format!("{what}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("")
        ))
    }
}

/// `(coalesced wakeups, every other dropped event)`; an unreadable object counts as total loss.
pub fn split_drop_counters(counters: &Value) -> (u64, u64) {
    let Some(map) = counters.as_object() else {
        return (0, u64::MAX);
    };
    let replaced = map
        .get("wakeup_data_replaced_entries")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let others = map
        .iter()
        .filter(|(name, _)| name.as_str() != "wakeup_data_replaced_entries")
        .filter_map(|(_, value)| value.as_u64())
        .sum();
    (replaced, others)
}

fn summarize_recording(raw: &Path) -> Result<RecordSummary, String> {
    use stutter::api::recorder::SessionFile;
    let bytes = fs::read(raw.join("session.json")).map_err(|e| format!("no session.json: {e}"))?;
    let session: SessionFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let counters = serde_json::to_value(&session.core.drop_counters).unwrap_or(Value::Null);
    let (replaced_wakeups, drops) = split_drop_counters(&counters);
    let frames = fs::read_to_string(raw.join("frame_correlation.json"))
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count() as u64)
        .unwrap_or(0);
    Ok(RecordSummary {
        valid: true,
        drops,
        replaced_wakeups,
        write_errors: session.core.event_stream_write_errors,
        frame_samples: frames,
    })
}

/// Parameters of the frozen scenario set that every host in one campaign shares.
pub fn vkmark_of(set: &ScenarioSet) -> super::scenarios::model::VkmarkParams {
    set.vkmark.clone()
}
