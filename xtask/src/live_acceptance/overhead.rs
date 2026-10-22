//! Profiler overhead characterization (A..F probe sets), repeated and counterbalanced
//! with the project's own order mechanism (`tune --order`). Unavailable probe sets are
//! reported as DEGRADED, never synthesized, and no acceptable-overhead target is chosen:
//! results are bounded claims tied to workload, kernel, probe set, commit and variation.
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

use stutter::api::{
    autotune::runnable_signals_for_target,
    recorder::{IntervalRecord, SessionFile},
    tune_recommendation::counterbalanced_order,
};

use super::{evidence::LiveAcceptanceSuite, overhead_external::ExternalWorkload, *};

pub(super) const CONFIGS: [(&str, &str); 6] = [
    ("A", "profiler disabled"),
    ("B", "scheduler core only"),
    ("C", "scheduler + IRQ"),
    ("D", "scheduler + block I/O"),
    ("E", "scheduler + display/GPU evidence"),
    ("F", "full currently supported observation set"),
];
const BASE: [&str; 5] = [
    "--no-hwmon",
    "--no-cpu-freq",
    "--no-faults",
    "--no-block-io",
    "--no-stat-wait",
];

/// `record` flags for a configuration; `None` means no profiler process at all.
pub(super) fn record_flags(id: &str, irq: Option<u32>) -> anyhow::Result<Option<Vec<String>>> {
    let base = BASE
        .iter()
        .map(|flag| (*flag).to_owned())
        .collect::<Vec<_>>();
    let swap = |flags: Vec<String>, remove: &[&str], add: &[&str]| {
        flags
            .into_iter()
            .filter(|flag| !remove.contains(&flag.as_str()))
            .chain(add.iter().map(|flag| (*flag).to_owned()))
            .collect::<Vec<_>>()
    };
    let irq_flags = |flags: Vec<String>| -> anyhow::Result<Vec<String>> {
        let irq = irq.context("no IRQ with activity available for the IRQ probe set")?;
        Ok(flags
            .into_iter()
            .chain([
                "--irq-latency".to_owned(),
                "--irq".to_owned(),
                irq.to_string(),
            ])
            .collect())
    };
    Ok(match id {
        "A" => None,
        "B" => Some(base),
        "C" => Some(irq_flags(base)?),
        "D" => Some(swap(base, &["--no-block-io"], &["--block-io"])),
        "E" => Some(swap(
            base,
            &["--no-hwmon"],
            &["--hwmon", "--gpu-engine-sampling"],
        )),
        "F" => {
            let all = swap(
                base,
                &BASE,
                &[
                    "--hwmon",
                    "--cpu-freq",
                    "--faults",
                    "--block-io",
                    "--stat-wait",
                    "--runtime-slices",
                ],
            );
            Some(irq_flags(all)?)
        }
        other => anyhow::bail!("unknown overhead configuration {other}"),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stats {
    pub n: usize,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub q1: f64,
    pub q3: f64,
}

fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let (low, high) = (position.floor() as usize, position.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (position - low as f64)
}

pub(super) fn summarize(values: &[f64]) -> Option<Stats> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(Stats {
        n: sorted.len(),
        median: quantile(&sorted, 0.5),
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        q1: quantile(&sorted, 0.25),
        q3: quantile(&sorted, 0.75),
    })
}

/// Throughput change relative to the profiler-disabled median (positive = slower).
pub(super) fn throughput_overhead_percent(disabled_median: f64, median: f64) -> Option<f64> {
    (disabled_median > 0.0).then(|| (disabled_median - median) / disabled_median * 100.0)
}

pub(super) fn cpu_percent(cpu_ticks: u64, wall_seconds: f64, ticks_per_second: f64) -> f64 {
    cpu_ticks as f64 / ticks_per_second / wall_seconds * 100.0
}

#[derive(Clone, Debug)]
pub(super) struct Sample {
    pub throughput: f64,
    pub profiler_cpu_percent: Option<f64>,
    pub profiler_rss_kib: Option<f64>,
    pub event_rate: Option<f64>,
    pub drops: Option<f64>,
    pub write_errors: Option<f64>,
    pub runnable_p99_ns: Option<f64>,
    pub runnable_over_5ms_rate: Option<f64>,
}

fn discover_irq() -> Option<u32> {
    fs::read_to_string("/proc/interrupts")
        .ok()?
        .lines()
        .skip(1)
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let irq = parts.next()?.strip_suffix(':')?.parse::<u32>().ok()?;
            parts
                .next()?
                .parse::<u64>()
                .ok()
                .filter(|count| *count > 0)
                .map(|_| irq)
        })
}

fn clock_ticks() -> f64 {
    // SAFETY-free: USER_HZ is 100 on every Linux ABI this suite supports; recorded below.
    100.0
}

fn read_profiler_sample(pid: u32) -> Option<(u64, f64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    let rss = fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| {
            value
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<f64>()
                .ok()
        })?;
    Some((ticks, rss))
}

/// The workload one profiler configuration is measured against.
#[derive(Clone, Copy)]
pub(super) enum Workload<'a> {
    /// The harness' own CPU-pinned synthetic game-path workload (`controlled_real`).
    Disposable { game: &'a Path, cpu: u32 },
    /// An operator-supplied repeatable fixed-work command (`real_workload`).
    External(&'a ExternalWorkload),
}

pub(super) fn run_one(
    binary: &Path,
    workload: Workload<'_>,
    dir: &Path,
    flags: Option<&[String]>,
    seconds: u64,
) -> anyhow::Result<Sample> {
    fs::create_dir_all(dir)?;
    let ready = dir.join("workload-ready.json");
    let workload_seconds = seconds + 4;
    let workload_started = Instant::now();
    let mut child = match workload {
        Workload::Disposable { game, cpu } => OwnedChild(
            Command::new("taskset")
                .args(["-c", &cpu.to_string()])
                .arg(game)
                .args([
                    "live-workload",
                    "--seconds",
                    &workload_seconds.to_string(),
                    "--ready",
                ])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(fs::File::create(dir.join("workload.stderr"))?)
                .spawn()?,
        ),
        Workload::External(external) => external.spawn(dir)?,
    };
    if matches!(workload, Workload::Disposable { .. }) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            ensure!(
                child.0.try_wait()?.is_none() && Instant::now() < deadline,
                "workload not ready"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let (pid, ticks) = (child.0.id(), start_ticks(child.0.id())?);
    let mut last = None;
    let recordings = dir.join("recording");
    let started = Instant::now();
    if let Some(flags) = flags {
        let mut record = args(&[
            "record",
            "--tree-pid",
            &pid.to_string(),
            "--duration",
            &seconds.to_string(),
            "--summary-ms",
            "250",
            "--out-dir",
        ]);
        record.push(recordings.to_string_lossy().into_owned());
        record.extend(flags.iter().cloned());
        let mut profiler = OwnedChild(
            Command::new(binary)
                .args(&record)
                .stdout(fs::File::create(dir.join("record.stdout"))?)
                .stderr(fs::File::create(dir.join("record.stderr"))?)
                .spawn()?,
        );
        let limit = Instant::now() + Duration::from_secs(seconds + 20);
        loop {
            let child = &mut profiler.0;
            if let Some(status) = child.try_wait()? {
                ensure!(
                    status.success(),
                    "record exited with {status}; see record.stderr"
                );
                break;
            }
            if let Some(sample) = read_profiler_sample(child.id()) {
                last = Some(sample);
            }
            ensure!(Instant::now() < limit, "record did not finish");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let throughput = match workload {
        Workload::Disposable { .. } => {
            let limit = Instant::now() + Duration::from_secs(workload_seconds + 10);
            while child.0.try_wait()?.is_none() {
                ensure!(Instant::now() < limit, "workload did not finish");
                std::thread::sleep(Duration::from_millis(50));
            }
            let counts: Value = serde_json::from_slice(&fs::read(ready.with_extension("counts"))?)?;
            counts["busy_iterations"]
                .as_u64()
                .context("busy iterations")? as f64
                / workload_seconds as f64
        }
        // Fixed work: throughput is work units over the whole run's elapsed time.
        Workload::External(external) => {
            external.wait(&mut child)?;
            external.throughput(workload_started.elapsed().as_secs_f64())
        }
    };
    let mut sample = Sample {
        throughput,
        profiler_cpu_percent: last.map(|(ticks, _)| cpu_percent(ticks, wall, clock_ticks())),
        profiler_rss_kib: last.map(|(_, rss)| rss),
        event_rate: None,
        drops: None,
        write_errors: None,
        runnable_p99_ns: None,
        runnable_over_5ms_rate: None,
    };
    if flags.is_some() {
        let recording = evidence::recording_dir(&recordings)?;
        let session: SessionFile =
            serde_json::from_slice(&fs::read(recording.join("session.json"))?)?;
        let records = fs::read_to_string(recording.join("interval.json"))?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<IntervalRecord>)
            .collect::<Result<Vec<_>, _>>()?;
        let events = records
            .iter()
            .filter(|r| r.process_pid == Some(pid))
            .map(|r| r.samples)
            .sum::<u64>();
        sample.event_rate = Some(events as f64 / seconds as f64);
        let drops = serde_json::to_value(&session.core.drop_counters)?;
        sample.drops = Some(drops.as_object().map_or(0.0, |m| {
            m.values().filter_map(Value::as_u64).sum::<u64>() as f64
        }));
        sample.write_errors = Some(session.core.event_stream_write_errors as f64);
        let signals = runnable_signals_for_target(&records, pid, ticks);
        sample.runnable_p99_ns = signals.runnable_latency_p99_ns.map(|v| v as f64);
        sample.runnable_over_5ms_rate = signals
            .runnable_over_5ms
            .zip(signals.runnable_samples.filter(|s| *s > 0))
            .map(|(over, samples)| over as f64 / samples as f64);
    }
    Ok(sample)
}

fn series(samples: &[&Sample], select: fn(&Sample) -> Option<f64>) -> Value {
    let values = samples.iter().filter_map(|s| select(s)).collect::<Vec<_>>();
    match summarize(&values) {
        Some(s) => json!({"n":s.n,"median":s.median,"min":s.min,"max":s.max,"q1":s.q1,"q3":s.q3}),
        None => Value::Null,
    }
}

pub(super) fn run(
    root: &Path,
    out: &Path,
    binary: &Path,
    seconds: u64,
    rounds: u32,
    seed: Option<u64>,
    mut external: Option<ExternalWorkload>,
) -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "linux"), "live acceptance requires Linux");
    let evidence_kind = if external.is_some() {
        "real_workload"
    } else {
        "controlled_real"
    };
    fs::create_dir(out).context("acceptance output must be a new directory")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(binary);
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated overhead defaults.\n",
    )?;
    let mut stage = "preflight";
    let mut cases = Vec::new();
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata_with_kind(
            root,
            &out,
            &binary,
            LiveAcceptanceSuite::Overhead,
            evidence_kind,
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
        let cpu = allowed_cpu()?;
        let irq = discover_irq();
        let game = if external.is_none() {
            let dir = out.join("steamapps/common/StutterOverhead");
            fs::create_dir_all(&dir)?;
            let target = dir.join("StutterOverhead");
            fs::copy(std::env::current_exe()?, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
            Some(target)
        } else {
            None
        };
        let warm_up_seconds = match external.as_mut() {
            Some(workload) => Some(workload.warm_up(&out.join("warm-up"), seconds)?),
            None => None,
        };
        let workload_identity = match &external {
            Some(workload) => workload.identity()?,
            None => json!({"label":"disposable game-path workload","declared_by":"harness"}),
        };
        stage = "runs";
        let mut samples: Vec<Vec<Sample>> = (0..CONFIGS.len()).map(|_| Vec::new()).collect();
        let mut degraded: BTreeMap<&str, String> = BTreeMap::new();
        let mut order_log = Vec::new();
        for round in 1..=rounds {
            let order = counterbalanced_order(CONFIGS.len(), round, seed);
            order_log.push(json!({"round":round,"order":order.iter().map(|i| CONFIGS[*i].0).collect::<Vec<_>>()}));
            for (position, index) in order.into_iter().enumerate() {
                let (id, _) = CONFIGS[index];
                if degraded.contains_key(id) {
                    continue;
                }
                let attempt = record_flags(id, irq).and_then(|flags| {
                    let dir = out.join(format!("runs/round{round:02}-pos{position}-{id}"));
                    let workload = match (&external, &game) {
                        (Some(external), _) => Workload::External(external),
                        (None, Some(game)) => Workload::Disposable { game, cpu },
                        (None, None) => anyhow::bail!("no workload configured"),
                    };
                    run_one(&binary, workload, &dir, flags.as_deref(), seconds)
                });
                match attempt {
                    Ok(sample) => samples[index].push(sample),
                    Err(error) => {
                        degraded.insert(id, format!("{error:#}"));
                    }
                }
            }
        }
        stage = "analysis";
        ensure!(
            samples[0].len() == rounds as usize && samples[1].len() == rounds as usize,
            "the profiler-disabled and scheduler-core baselines must complete every round: {degraded:?}"
        );
        let disabled = summarize(&samples[0].iter().map(|s| s.throughput).collect::<Vec<_>>())
            .context("baseline throughput")?;
        let mut configs = Vec::new();
        let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")?
            .trim()
            .to_owned();
        for (index, (id, label)) in CONFIGS.iter().enumerate() {
            let refs = samples[index].iter().collect::<Vec<_>>();
            if refs.is_empty() {
                configs.push(
                    json!({"id":id,"label":label,"status":"DEGRADED","reason":degraded.get(id)}),
                );
                continue;
            }
            cases.push(json!({"name":id,"status":"PASS"}));
            let throughput = series(&refs, |s| Some(s.throughput));
            let median = throughput["median"].as_f64().unwrap_or(0.0);
            let overhead = throughput_overhead_percent(disabled.median, median);
            configs.push(json!({"id":id,"label":label,"status":"PASS","runs":refs.len(),
                "workload_throughput_iterations_per_s":throughput,"throughput_overhead_percent_vs_disabled":overhead,
                "profiler_cpu_percent":series(&refs, |s| s.profiler_cpu_percent),
                "profiler_rss_kib":series(&refs, |s| s.profiler_rss_kib),
                "events_per_s":series(&refs, |s| s.event_rate),
                "dropped_events":series(&refs, |s| s.drops),
                "artifact_write_errors":series(&refs, |s| s.write_errors),
                "runnable_p99_ns":series(&refs, |s| s.runnable_p99_ns),
                "runnable_over_5ms_rate":series(&refs, |s| s.runnable_over_5ms_rate),
                "bounded_claim":format!("On kernel {kernel} with probe set '{label}', median workload throughput differs by {} % versus profiler-disabled (IQR {:.0}-{:.0} iterations/s, n={}); no acceptable-overhead target is asserted",
                    overhead.map_or("n/a".to_owned(), |v| format!("{v:.2}")),
                    throughput["q1"].as_f64().unwrap_or(0.0), throughput["q3"].as_f64().unwrap_or(0.0), refs.len())}));
        }
        Ok(
            json!({"workload":workload_identity,"warm_up_seconds":warm_up_seconds,
            "cache_policy":"same workload, settings and scene in every run; one unmeasured warm-up run before the rounds; page cache not dropped by the harness",
            "configurations":configs,"rounds":rounds,"seconds":seconds,
            "order_strategy":seed.map_or("alternating".to_owned(), |s| format!("seed:{s}")),"order":order_log,
            "irq_used":irq,"clock_ticks_per_second":clock_ticks(),"degraded":degraded}),
        )
    })();
    let status = if result.is_ok() {
        "PASS"
    } else if stage == "preflight" {
        "BLOCKED"
    } else {
        "FAIL"
    };
    let receipt = json!({"schema_version":1,"status":status,"stage":stage,"evidence_kind":evidence_kind,"cases":cases,
    "measurements":result.as_ref().ok(),"error":result.as_ref().err().map(|e| format!("{e:#}")),
    "scope":if external.is_some() {
        "profiler overhead on an operator-declared repeatable fixed-work real workload (throughput = work units over elapsed time); counterbalanced repeated runs; profiler side (CPU, RSS, event rate, drops, write errors) and workload side (throughput, runnable p99, >5 ms rate of the launched process)"
    } else {
        "profiler overhead on a single-CPU-pinned synthetic game-path workload; counterbalanced repeated runs; profiler side (CPU, RSS, event rate, drops, write errors) and workload side (throughput, runnable p99, >5 ms rate)"
    },
    "not_claimed":if external.is_some() {
        json!(["that the workload is representative (the operator declares it)","other workloads","frame-time metrics","any acceptable-overhead target","other kernels","DEGRADED probe sets are not measured","portable verified bundle until exported"])
    } else {
        json!(["overhead on real games or other workloads","frame-time metrics (the synthetic workload has none)","any acceptable-overhead target","other kernels","DEGRADED probe sets are not measured","portable verified bundle until exported"])
    }});
    evidence::write_result(&out, LiveAcceptanceSuite::Overhead, receipt)?;
    println!("{status}: overhead evidence {}", out.display());
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_sets_add_only_what_they_name() {
        assert!(record_flags("A", None).unwrap().is_none());
        let b = record_flags("B", None).unwrap().unwrap();
        assert!(b.contains(&"--no-block-io".to_owned()) && !b.contains(&"--block-io".to_owned()));
        let d = record_flags("D", None).unwrap().unwrap();
        assert!(d.contains(&"--block-io".to_owned()) && !d.contains(&"--no-block-io".to_owned()));
        assert!(d.contains(&"--no-hwmon".to_owned()));
        let c = record_flags("C", Some(9)).unwrap().unwrap();
        assert!(c.windows(2).any(|w| w == ["--irq", "9"]));
        // Unavailable probe sets are errors (-> DEGRADED), never fabricated.
        assert!(record_flags("C", None).is_err());
        assert!(record_flags("F", None).is_err());
        let f = record_flags("F", Some(9)).unwrap().unwrap();
        assert!(!f.iter().any(|flag| flag.starts_with("--no-")));
        assert!(record_flags("Z", None).is_err());
    }

    #[test]
    fn summary_statistics_and_overheads() {
        let stats = summarize(&[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();
        assert_eq!(
            (stats.n, stats.median, stats.q1, stats.q3),
            (5, 30.0, 20.0, 40.0)
        );
        assert!(summarize(&[]).is_none() && summarize(&[f64::NAN]).is_none());
        assert_eq!(throughput_overhead_percent(100.0, 97.5), Some(2.5));
        assert_eq!(throughput_overhead_percent(0.0, 1.0), None);
        assert!((cpu_percent(250, 10.0, 100.0) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn every_configuration_visits_every_position_equally_over_two_blocks() {
        let mut counts = [[0_u32; 6]; 6];
        for round in 1..=12 {
            for (position, index) in counterbalanced_order(CONFIGS.len(), round, None)
                .into_iter()
                .enumerate()
            {
                counts[index][position] += 1;
            }
        }
        assert!(counts.iter().flatten().all(|count| *count == 2));
        assert_ne!(
            counterbalanced_order(6, 1, Some(1)),
            counterbalanced_order(6, 1, Some(2))
        );
    }
}
