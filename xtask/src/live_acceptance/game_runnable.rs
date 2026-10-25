//! Controlled real `GameRunnableLatency` acceptance. The target is a disposable
//! process whose executable lives under a `steamapps/common` path, so the PRODUCTION
//! task classifier (not a test hook) must recognise it as a game. Two scenarios on the
//! same workload: a low-contention baseline and one with scheduler contention pinned to
//! the same CPU. The objective signal is computed from the recording by production
//! code (`runnable_signals_for_target`); nothing is injected.
use std::os::unix::fs::PermissionsExt;

use stutter::api::{
    autotune::{
        MINIMUM_RUNNABLE_SAMPLES, ObjectiveSignals, game_runnable_unavailable_reason,
        runnable_signals_for_target,
    },
    process_tree::TaskClass,
    recorder::{IntervalRecord, SessionFile},
};

use super::{evidence::LiveAcceptanceSuite, *};

const CONTENDERS: usize = 3;

pub(super) struct Scenario {
    pub name: &'static str,
    pub pid: u32,
    pub ticks: u64,
    pub records: Vec<IntervalRecord>,
    pub session: SessionFile,
}

fn is_primary_game_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game
            | TaskClass::GameRenderThread
            | TaskClass::GameWorkerThread
            | TaskClass::GameHelper
            | TaskClass::WineServer
    )
}

/// Measurements for one scenario; every requirement of the plan is a hard check.
pub(super) fn measurements(scenario: &Scenario) -> anyhow::Result<(ObjectiveSignals, Value)> {
    let tasks = scenario
        .session
        .tasks
        .iter()
        .filter(|task| {
            task.process_pid == Some(scenario.pid)
                && task.process_starttime_ticks == Some(scenario.ticks)
        })
        .collect::<Vec<_>>();
    ensure!(
        tasks.iter().any(|task| is_primary_game_class(task.class)),
        "{}: production classification did not recognise a latency-sensitive game task population",
        scenario.name
    );
    let signals = runnable_signals_for_target(&scenario.records, scenario.pid, scenario.ticks);
    if let Some(reason) = game_runnable_unavailable_reason(&signals) {
        bail_unavailable(scenario.name, &reason)?;
    }
    let samples = signals
        .runnable_samples
        .context("runnable sample count missing")?;
    ensure!(
        samples >= MINIMUM_RUNNABLE_SAMPLES,
        "{}: {samples} runnable samples < objective minimum {MINIMUM_RUNNABLE_SAMPLES}",
        scenario.name
    );
    let p99 = signals
        .runnable_latency_p99_ns
        .context("runnable p99 missing")?;
    ensure!(p99 > 0, "{}: runnable p99 is not positive", scenario.name);
    let over_5ms = signals
        .runnable_over_5ms
        .context("severe-tail rate unavailable")?;
    let classes = tasks
        .iter()
        .map(|task| format!("{:?}", task.class))
        .collect::<std::collections::BTreeSet<_>>();
    let value = json!({
        "name": scenario.name, "target_pid": scenario.pid, "target_starttime_ticks": scenario.ticks,
        "task_classes": classes, "runnable_samples": samples, "runnable_p99_ns": p99,
        "runnable_max_ns": signals.runnable_latency_max_ns, "runnable_over_5ms": over_5ms,
        "runnable_over_5ms_rate": over_5ms as f64 / samples as f64,
        "signal_source": signals.runnable_latency_source, "signal_quality": "direct",
    });
    Ok((signals, value))
}

fn bail_unavailable(name: &str, reason: &str) -> anyhow::Result<()> {
    anyhow::bail!("{name}: direct runnable-latency signal unavailable: {reason}")
}

/// The degraded scenario must be directionally worse on the same objective metric.
/// Thresholds stay provisional: only the direction is asserted here.
pub(super) fn compare(baseline: &Value, degraded: &Value) -> anyhow::Result<Value> {
    let (b, d) = (
        baseline["runnable_p99_ns"]
            .as_u64()
            .context("baseline p99")?,
        degraded["runnable_p99_ns"]
            .as_u64()
            .context("degraded p99")?,
    );
    ensure!(
        d > b,
        "degraded runnable p99 {d} ns is not worse than baseline {b} ns"
    );
    Ok(
        json!({"baseline_p99_ns":b,"degraded_p99_ns":d,"p99_ratio":d as f64 / b as f64,
        "baseline_over_5ms_rate":baseline["runnable_over_5ms_rate"],
        "degraded_over_5ms_rate":degraded["runnable_over_5ms_rate"],"directionally_worse":true,
        "thresholds":"provisional; effect-size thresholds are calibrated separately"}),
    )
}

pub(super) fn install_game_binary(out: &Path) -> anyhow::Result<PathBuf> {
    let dir = out.join("steamapps/common/StutterAcceptanceGame");
    fs::create_dir_all(&dir)?;
    let target = dir.join("StutterAcceptanceGame");
    fs::copy(std::env::current_exe()?, &target)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
    Ok(target)
}

/// Scheduler contention applied to the target's CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Contention {
    /// N ordinary CPU hogs (unbounded, non-deterministic delay).
    Hogs(usize),
    /// A SCHED_FIFO contender that holds the CPU for `ms` out of every 20 ms: the target
    /// can be blocked for up to `ms`, a controlled degradation level.
    Burst(u64),
}

pub(super) fn spawn_contenders(
    cpu: u32,
    seconds: u64,
    dir: &Path,
    contention: Contention,
) -> anyhow::Result<Vec<OwnedChild>> {
    let mut children = Vec::new();
    let executable = std::env::current_exe()?;
    match contention {
        Contention::Hogs(count) => {
            for index in 0..count {
                let ready = dir.join(format!("contender-{index}.json"));
                children.push(OwnedChild(
                    Command::new("taskset")
                        .args(["-c", &cpu.to_string()])
                        .arg(&executable)
                        .args([
                            "live-workload",
                            "--seconds",
                            &(seconds + 20).to_string(),
                            "--ready",
                        ])
                        .arg(&ready)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .unprivileged()
                        .spawn()?,
                ));
            }
        }
        Contention::Burst(0) => {}
        // A campaign with a validation runner requests the contender as a typed action; the
        // runner owns the process and ends it after the window.
        Contention::Burst(ms) if super::runner_burst::RunnerEnv::from_process().is_some() => {
            if let Some(env) = super::runner_burst::RunnerEnv::from_process() {
                env.start_burst(cpu, ms, 20, seconds + 20)?;
            }
        }
        Contention::Burst(ms) => children.push(OwnedChild(
            Command::new("chrt")
                .args(["-f", "50", "taskset", "-c", &cpu.to_string()])
                .arg(&executable)
                .args([
                    "live-burst",
                    "--burst-ms",
                    &ms.to_string(),
                    "--period-ms",
                    "20",
                    "--seconds",
                    &(seconds + 20).to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        )),
    }
    Ok(children)
}

pub(super) fn run_scenario(
    binary: &Path,
    game: &Path,
    out: &Path,
    name: &'static str,
    cpu: u32,
    seconds: u64,
    contention: Contention,
) -> anyhow::Result<(Scenario, Value)> {
    let dir = out.join(name);
    fs::create_dir(&dir)?;
    let hogs = spawn_contenders(cpu, seconds, &dir, contention)?;
    let ready = dir.join("workload-ready.json");
    let mut child = OwnedChild(
        Command::new("taskset")
            .args(["-c", &cpu.to_string()])
            .arg(game)
            .args([
                "live-workload",
                "--seconds",
                &(seconds + 20).to_string(),
                "--ready",
            ])
            .arg(&ready)
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.join("workload.stderr"))?)
            .unprivileged()
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        ensure!(
            child.0.try_wait()?.is_none(),
            "{name}: workload exited before readiness"
        );
        ensure!(
            Instant::now() < deadline,
            "{name}: workload readiness timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid = child.0.id();
    let ticks = start_ticks(pid)?;
    let recordings = dir.join("recording");
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
        binary,
        &record_args,
        &dir,
        "record",
        Duration::from_secs(seconds + 15),
    )?;
    ensure!(
        child.0.try_wait()?.is_none() && start_ticks(pid)? == ticks,
        "{name}: target identity did not survive recording"
    );
    drop(child);
    drop(hogs);
    let recording = evidence::recording_dir(&recordings)?;
    // Histogram completeness, zero drops, identity and schema checks.
    let generic = evidence::validate(&recording, pid, ticks)?;
    let run = recording.to_string_lossy().into_owned();
    command(
        binary,
        &args(&["validate", &run, "--json"]),
        &dir,
        "validate",
        Duration::from_secs(20),
    )?;
    let validation: Value = serde_json::from_slice(&fs::read(dir.join("validate.stdout"))?)?;
    ensure!(
        validation["passed"] == true,
        "{name}: artifact validator failed"
    );
    command(
        binary,
        &args(&["report", &run, "--analysis-json"]),
        &dir,
        "analysis",
        Duration::from_secs(20),
    )?;
    let analysis: Value = serde_json::from_slice(&fs::read(dir.join("analysis.stdout"))?)?;
    ensure!(
        analysis.get("cluster_analysis").is_some(),
        "{name}: diagnosis analysis missing"
    );
    let html = dir.join("report.html");
    command(
        binary,
        &args(&["report", &run, "--html", &html.to_string_lossy()]),
        &dir,
        "html",
        Duration::from_secs(20),
    )?;
    ensure!(fs::metadata(&html)?.len() > 0, "{name}: empty HTML report");
    command(
        binary,
        &args(&["advisor", "--run", &run, "--json"]),
        &dir,
        "advisor",
        Duration::from_secs(20),
    )?;
    serde_json::from_slice::<Value>(&fs::read(dir.join("advisor.stdout"))?)?;
    let session: SessionFile = serde_json::from_slice(&fs::read(recording.join("session.json"))?)?;
    let records = fs::read_to_string(recording.join("interval.json"))?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<IntervalRecord>)
        .collect::<Result<Vec<_>, _>>()?;
    let scenario = Scenario {
        name,
        pid,
        ticks,
        records,
        session,
    };
    let (_, mut value) = measurements(&scenario)?;
    value["generic_artifact_validation"] = generic;
    Ok((scenario, value))
}

pub(super) fn run(root: &Path, out: &Path, binary: &Path, seconds: u64) -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "linux"), "live acceptance requires Linux");
    fs::create_dir(out).context("acceptance output must be a new directory")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(binary);
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated game-runnable defaults.\n",
    )?;
    let mut stage = "preflight";
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(root, &out, &binary, LiveAcceptanceSuite::GameRunnable)?;
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
        let cpu = allowed_cpu()?;
        let game = install_game_binary(&out)?;
        stage = "baseline";
        let (_, baseline) = run_scenario(
            &binary,
            &game,
            &out,
            "baseline",
            cpu,
            seconds,
            Contention::Hogs(0),
        )?;
        stage = "degraded";
        let (_, degraded) = run_scenario(
            &binary,
            &game,
            &out,
            "degraded",
            cpu,
            seconds,
            Contention::Hogs(CONTENDERS),
        )?;
        stage = "comparison";
        let comparison = compare(&baseline, &degraded)?;
        Ok(
            json!({"baseline":baseline,"degraded":degraded,"comparison":comparison,"contenders":CONTENDERS}),
        )
    })();
    let status = if result.is_ok() {
        "PASS"
    } else if stage == "preflight" {
        "BLOCKED"
    } else {
        "FAIL"
    };
    let receipt = json!({"schema_version":1,"status":status,"stage":stage,"evidence_kind":"controlled_real",
        "cases":[{"name":"baseline_vs_degraded","status":status}],
        "measurements":result.as_ref().ok(),"error":result.as_ref().err().map(|e| format!("{e:#}")),
        "scope":"production game classification and merged runnable-latency objective signal on a disposable game-path workload, low contention versus pinned CPU contention",
        "not_claimed":["real game workload","effect-size thresholds (provisional)","kernel compatibility beyond this host","mutation benefit","portable verified bundle until exported"]});
    evidence::write_result(&out, LiveAcceptanceSuite::GameRunnable, receipt)?;
    println!("{status}: game-runnable evidence {}", out.display());
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use stutter::api::recorder::{LatencyHistogram, SESSION_SCHEMA_VERSION, SessionTask};

    use super::*;

    /// Records `samples` events of `latency_ns` through the real histogram so every
    /// production consistency rule (counts, tails, maximum) holds.
    fn histogram_record(pid: u32, samples: u64, latency_ns: u64) -> IntervalRecord {
        let mut histogram = LatencyHistogram::new();
        for _ in 0..samples {
            histogram.record(latency_ns);
        }
        let buckets = histogram.snapshot();
        let tail = |threshold: u64| -> u64 {
            buckets
                .iter()
                .filter(|bucket| bucket.upper_bound_ns.is_none_or(|upper| upper > threshold))
                .map(|bucket| bucket.count)
                .sum()
        };
        let mut value = serde_json::to_value(IntervalRecord {
            process_pid: Some(pid),
            class: TaskClass::Game,
            samples,
            max_ns: latency_ns,
            over_1ms: tail(1_000_000),
            over_2ms: tail(2_000_000),
            over_5ms: tail(5_000_000),
            ..Default::default()
        })
        .unwrap();
        value["histogram"] = serde_json::to_value(buckets).unwrap();
        serde_json::from_value(value).unwrap()
    }

    fn scenario(name: &'static str, samples: u64, latency_ns: u64, class: TaskClass) -> Scenario {
        let mut session = SessionFile::default();
        session.core.schema_version = SESSION_SCHEMA_VERSION;
        session.tasks.push(SessionTask {
            process_pid: Some(7),
            process_starttime_ticks: Some(70),
            class,
            comm: "accept-wake".into(),
            ..Default::default()
        });
        Scenario {
            name,
            pid: 7,
            ticks: 70,
            records: vec![histogram_record(7, samples, latency_ns)],
            session,
        }
    }

    #[test]
    fn requirements_are_hard_checks() {
        assert!(measurements(&scenario("ok", 5_000, 2_000_000, TaskClass::Game)).is_ok());
        // Production classification must recognise a game population.
        assert!(measurements(&scenario("c", 5_000, 2_000_000, TaskClass::Compiler)).is_err());
        // Objective minimum sample count.
        let few = scenario(
            "n",
            MINIMUM_RUNNABLE_SAMPLES - 1,
            2_000_000,
            TaskClass::Game,
        );
        assert!(measurements(&few).is_err());
        // Wrong identity yields no population at all.
        let mut wrong = scenario("w", 5_000, 2_000_000, TaskClass::Game);
        wrong.ticks = 71;
        assert!(measurements(&wrong).is_err());
    }

    #[test]
    fn degraded_must_be_directionally_worse() {
        let (_, low) = measurements(&scenario("low", 5_000, 500_000, TaskClass::Game)).unwrap();
        let (_, high) =
            measurements(&scenario("high", 5_000, 20_000_000, TaskClass::Game)).unwrap();
        let comparison = compare(&low, &high).unwrap();
        assert_eq!(comparison["directionally_worse"], true);
        assert!(compare(&high, &low).is_err());
        assert!(compare(&low, &low).is_err());
        assert!(high["runnable_over_5ms"].as_u64().unwrap() > 0);
    }
}
