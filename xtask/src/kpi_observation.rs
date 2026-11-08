//! A bounded, unprivileged fixed-work demonstration of runtime KPI ingestion and raw-block analysis.
//! Monitor ticks are harness-supplied: this is not installed-service or scheduler qualification.
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use clap::Subcommand;
use stutter::api::{
    autotune::runtime::{AutotuneRuntime, AutotuneRuntimeConfig},
    experiments::{ExperimentDesign, KpiObservationStudyConfig, StoppingRule},
    objective_source::{
        Aggregation, Direction, KpiBlockRequest, KpiContract, MetricClass, MetricSpec,
        ObservedKpiBlock, Sample, SampleKind, SourceBinding, monotonic_now_ns,
    },
    session_events::MonitorEvent,
};

const WORK: u64 = 100_000;
const WORK_ID: &str = "integer-fold-v1-100000";

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum KpiObservationCommand {
    /// Run eight independently completed, identical fixed-work executions, with no actuator.
    Run {
        #[arg(long)]
        out: PathBuf,
    },
    #[command(hide = true)]
    Producer {
        #[arg(long)]
        control: PathBuf,
        #[arg(long)]
        socket: PathBuf,
    },
}

fn fixed_work() -> u64 {
    (0..WORK).fold(0_u64, |value, index| {
        std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(index))
    })
}

fn producer(control: &Path, socket: &Path) -> anyhow::Result<()> {
    let mut control = UnixStream::connect(control)?;
    control.set_read_timeout(Some(Duration::from_secs(20)))?;
    control.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut commands = BufReader::new(control.try_clone()?);
    // Independently checked modulo-2^64 checksum of the frozen 100000-step recurrence.
    let expected = 17_641_615_109_599_008_432;
    let mut line = String::new();
    let mut source = None;
    for _ in 0..9 {
        line.clear();
        commands.by_ref().take(128).read_line(&mut line)?;
        if line == "stop\n" {
            return Ok(());
        }
        ensure!(line == "measure\n", "unknown bounded producer request");
        if source.is_none() {
            let stream = UnixStream::connect(socket)?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            source = Some(BufReader::new(stream));
        }
        let started = Instant::now();
        ensure!(fixed_work() == expected, "fixed-work correctness failed");
        let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
        let at = monotonic_now_ns()?;
        for (metric, value) in [
            ("fixed_work_ms", milliseconds),
            ("completed_units", WORK as f64),
        ] {
            let sample = Sample {
                metric: metric.into(),
                value,
                monotonic_ns: at,
                work_unit: Some(WORK_ID.into()),
                population: 1,
                kind: SampleKind::RawEvent,
            };
            let source = source.as_mut().context("source was not opened")?;
            writeln!(source.get_mut(), "{}", serde_json::to_string(&sample)?)?;
            let mut answer = String::new();
            source.by_ref().take(128).read_line(&mut answer)?;
            ensure!(answer == "ok\n", "KPI was refused: {answer}");
        }
        control.write_all(b"complete\n")?;
    }
    anyhow::bail!("producer request budget exhausted")
}

struct Producer {
    cancel: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<anyhow::Result<crate::process::supervisor::Supervised>>>,
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn spec(id: &str, unit: &str, direction: Direction) -> MetricSpec {
    MetricSpec {
        id: id.into(),
        unit: unit.into(),
        class: if id == "fixed_work_ms" {
            MetricClass::Latency
        } else {
            MetricClass::Throughput
        },
        direction,
        aggregation: Aggregation::Mean,
        max_age_ms: 5000,
        min_population: 1,
        desired_value: None,
        meaningful_effect_percent: 20.0,
        equivalent_within_percent: 5.0,
    }
}

fn tick(runtime: &mut AutotuneRuntime, elapsed_ms: u64) -> anyhow::Result<()> {
    let entry = runtime.on_event(MonitorEvent::Interval {
        elapsed_ms,
        records: Vec::new(),
        drop_counters: Default::default(),
    })?;
    ensure!(
        entry
            .as_ref()
            .is_none_or(|entry| entry.decision == "observed"),
        "unexpected action decision"
    );
    Ok(())
}

fn capture(
    runtime: &mut AutotuneRuntime,
    control: &mut UnixStream,
    out: &Path,
    pair: u32,
    arm: &str,
    context: &str,
    started: Instant,
) -> anyhow::Result<ObservedKpiBlock> {
    let from_ns = monotonic_now_ns()?;
    control.write_all(b"measure\n")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut reply = Vec::new();
    loop {
        ensure!(
            Instant::now() < deadline,
            "fixed-work completion deadline exceeded"
        );
        tick(runtime, started.elapsed().as_millis() as u64)?;
        let mut buffer = [0; 64];
        match control.read(&mut buffer) {
            Ok(0) => anyhow::bail!("producer exited before completion"),
            Ok(count) => {
                reply.extend_from_slice(&buffer[..count]);
                ensure!(reply.len() <= 64, "oversize completion");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        if reply.ends_with(b"\n") {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    ensure!(reply == b"complete\n", "invalid completed-work marker");
    let block = runtime.capture_kpi_block(KpiBlockRequest {
        block_id: format!("block-{pair}"),
        run_id: format!("run-{pair}-{arm}"),
        work_unit: WORK_ID.into(),
        context_fingerprint: context.into(),
        from_ns,
        to_ns: monotonic_now_ns()?,
    })?;
    // Single-use directory; raw data is persisted before the pair is admitted or analysed.
    let path = out.join(format!("block-{pair}-{arm}.json"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&block.artifact_bytes()?)?;
    file.sync_all()?;
    Ok(block)
}

fn run(out: &Path) -> anyhow::Result<()> {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(out)
        .context("output must be a new owner-only directory")?;
    let out = out.canonicalize()?;
    let control_path = out.join("control.sock");
    let listener = stutter::api::telemetry::bind_fresh(&out, "control.sock", unsafe_uid())
        .map_err(|error| anyhow::anyhow!("control socket: {error:?}"))?;
    listener.set_nonblocking(true)?;
    let executable = std::env::current_exe()?;
    let digest = stutter_core::hash::sha256_hex(&fs::read(&executable)?);
    let mut command = Command::new(&executable);
    command
        .args(["kpi-observation", "producer", "--control"])
        .arg(&control_path)
        .arg("--socket")
        .arg(out.join("kpi.sock"));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut policy = crate::process::supervisor::ExecutionPolicy::bounded(Duration::from_secs(45));
    policy.tee = false;
    policy.cancel = Some(Arc::clone(&cancel));
    let mut producer = Producer {
        cancel,
        task: Some(thread::spawn(move || {
            crate::process::supervisor::run(command, &policy)
        })),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut control = loop {
        ensure!(
            Instant::now() < deadline,
            "producer readiness deadline exceeded"
        );
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.into()),
        }
    };
    let peer = stutter::api::telemetry::kernel_peer_credentials(&control)?;
    ensure!(peer.uid == unsafe_uid(), "wrong producer UID");
    let live = stutter::api::telemetry::live_process(Path::new("/proc"), peer.pid)
        .context("producer exited")?;
    let owner = crate::process::ownership::current_owner(Path::new("/proc"))
        .context("supervisor identity missing")?;
    ensure!(
        live.ancestors.contains(&(owner.pid, owner.start_ticks)),
        "control peer is not the owned producer descendant"
    );
    let contract = KpiContract {
        binding: SourceBinding {
            uid: peer.uid,
            pid: peer.pid,
            start_ticks: live.start_ticks,
        },
        primary: spec("fixed_work_ms", "ms", Direction::LowerIsBetter),
        hard_guardrails: vec![spec("completed_units", "count", Direction::HigherIsBetter)],
        advisory_guardrails: Vec::new(),
    };
    let context = stutter_core::hash::sha256_hex(
        format!(
            "{digest}:{}:{WORK_ID}",
            fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        )
        .as_bytes(),
    );
    let design = KpiObservationStudyConfig {
        design: ExperimentDesign {
            schema_version: 1,
            objective: "Experimental completed fixed work".into(),
            metric: contract.primary.id.clone(),
            unit: contract.primary.unit.clone(),
            candidate: "identical arms; no actuator".into(),
            context_fingerprint: context.clone(),
            min_effect_percent: 20.0,
            equivalence_margin_percent: 5.0,
            seed: 42,
            block_ids: (0..4).map(|i| format!("block-{i}")).collect(),
            stopping: StoppingRule::FixedN {
                blocks: 4,
                confidence: 0.95,
            },
            warmup_seconds: 0,
            washout_seconds: 0,
        },
        contract: contract.clone(),
        work_unit: WORK_ID.into(),
        hard_guardrail_regression_percent: std::collections::BTreeMap::from([(
            "completed_units".into(),
            0.0,
        )]),
    };
    let contract_path = out.join("contract.json");
    fs::write(&contract_path, serde_json::to_vec_pretty(&contract)?)?;
    fs::write(out.join("design.json"), serde_json::to_vec_pretty(&design)?)?;
    let mut config = AutotuneRuntimeConfig::observe(None, Some(peer.pid), None);
    config.history_log = None;
    config.daemon_config.autotune.kpi_contract = Some(contract_path);
    config.daemon_config.autotune.kpi_socket_dir = Some(out.clone());
    let mut runtime = AutotuneRuntime::new(config);
    tick(&mut runtime, 0)?;
    runtime.begin_kpi_observation_study(design.clone())?;
    control.set_nonblocking(true)?;
    let started = Instant::now();
    let mut pairs = Vec::new();
    for index in 0_u32..4 {
        let (first, second) = if index.is_multiple_of(2) {
            ("baseline", "treatment")
        } else {
            ("treatment", "baseline")
        };
        let a = capture(
            &mut runtime,
            &mut control,
            &out,
            index,
            first,
            &context,
            started,
        )?;
        let b = capture(
            &mut runtime,
            &mut control,
            &out,
            index,
            second,
            &context,
            started,
        )?;
        let pair = if first == "baseline" { (a, b) } else { (b, a) };
        runtime.observe_kpi_pair(&pair.0, &pair.1)?;
        pairs.push(pair);
    }
    let evidence = runtime.analyse_kpi_observation_study(1)?;
    let refs: Vec<_> = pairs.iter().map(|(a, b)| (a, b)).collect();
    evidence.verify_captures(&design, &refs)?;
    fs::write(
        out.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    ensure!(
        !runtime.has_active_experiment() && !runtime.has_kept_actions(),
        "observation acquired action state"
    );
    control.write_all(b"stop\n")?;
    let task = producer.task.take().context("producer task missing")?;
    let result = task
        .join()
        .map_err(|_| anyhow::anyhow!("producer supervisor panicked"))??;
    ensure!(
        matches!(result.outcome, crate::process::supervisor::Outcome::Exited(status) if status.success()),
        "producer failed: {:?} {}",
        result.outcome,
        result.stderr
    );
    drop(runtime);
    drop(listener);
    fs::remove_file(control_path)?;
    ensure!(
        !out.join("kpi.sock").exists(),
        "KPI socket was not cleaned up"
    );
    fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema_version":1,"evidence_kind":"controlled_real","scope":"runtime KPI and independent-block statistics; harness-supplied monitor ticks, no installed-service or scheduler qualification","xtask_sha256":digest,"completed_runs":8,"source_bound":true,"raw_reconstruction_verified":true,"authorizes_mutation":false,"outstanding_owned_tokens":0,"decision":evidence.evidence.decision}),
        )?,
    )?;
    println!(
        "kpi-observation: completed 8 fixed-work runs; {:?}; no mutation authority",
        evidence.evidence.decision
    );
    Ok(())
}

fn unsafe_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub fn execute(command: KpiObservationCommand) -> anyhow::Result<()> {
    match command {
        KpiObservationCommand::Run { out } => run(&out),
        KpiObservationCommand::Producer { control, socket } => producer(&control, &socket),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn completed_work_has_the_independently_checked_checksum() {
        assert_eq!(fixed_work(), 17_641_615_109_599_008_432);
    }

    #[test]
    fn cli_requires_an_explicit_output_and_offers_no_arbitrary_workload_command() {
        assert!(crate::Cli::try_parse_from(["xtask", "kpi-observation", "run"]).is_err());
        assert!(
            crate::Cli::try_parse_from(["xtask", "kpi-observation", "run", "--out", "private-run"])
                .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "xtask",
                "kpi-observation",
                "run",
                "--out",
                "private-run",
                "--",
                "sh"
            ])
            .is_err()
        );
        let help = crate::Cli::try_parse_from(["xtask", "kpi-observation", "--help"])
            .unwrap_err()
            .to_string();
        assert!(help.contains("run"));
        assert!(!help.contains("producer"));
    }
}
