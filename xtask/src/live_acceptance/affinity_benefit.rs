//! One matched pair of the CPU-affinity benefit experiment, through the production live
//! controller. A disposable game-like tree runs on a contended CPU; the controller (the real
//! `autotune apply-low-risk` session, holding an exact-scope validation override) observes it,
//! classifies the situation from direct telemetry, asks the affinity provider for a candidate,
//! plans, applies it for real, measures the target's runnable-latency histogram again and keeps
//! or reverts by `GameRunnableLatency`. The populations the objective compared are read back from
//! the typed experiment evidence in the controller's own history; nothing is injected.
use std::{
    os::unix::process::ExitStatusExt,
    process::{Child, Command, Stdio},
};

use stutter::api::autotune::{
    AcceptanceMutation, AutotuneCommandInput, MutationOperationKind, ObjectiveKind,
    ValidationOverride, ValidationRun, autotune_command_blocking,
};

use super::*;

/// The contention scenario of a pair. The target starts on CPU `c0`; the profile moves it to `c1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Scenario {
    /// Hogs on `c0`, `c1` quiet: moving helps.
    Beneficial,
    /// The same hogs on both CPUs: moving cannot help.
    Neutral,
    /// Light contention on `c0`, heavy on `c1`: moving hurts.
    Regression,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::Beneficial => "beneficial",
            Self::Neutral => "neutral",
            Self::Regression => "regression",
        }
    }

    /// Scheduler contention on (`c0`, `c1`): a SCHED_FIFO contender holding the CPU for the given
    /// milliseconds of every 20, so the target's wakeups wait up to that long. Ordinary hogs do
    /// not delay a sleeper under EEVDF and would measure nothing.
    pub(super) fn contention(
        self,
        mutation: AcceptanceMutation,
    ) -> (game_runnable::Contention, game_runnable::Contention) {
        use game_runnable::Contention::{Burst, Hogs};
        match (mutation, self) {
            // A priority change can only help against contention that shares the CPU by weight:
            // ordinary hogs on the target's CPU. Neutral and regression keep the hogs visible as
            // offenders (on the other CPU) while the target's contention is a SCHED_FIFO burst no
            // priority change can touch, at two levels, so any keep there is a false keep. The
            // burst also makes the situation scheduler pressure, which the provider needs.
            (AcceptanceMutation::Nice, Self::Beneficial) => (Hogs(3), Burst(0)),
            (AcceptanceMutation::Nice, Self::Neutral) => (Burst(8), Hogs(3)),
            (AcceptanceMutation::Nice, Self::Regression) => (Burst(2), Hogs(3)),
            (_, Self::Beneficial) => (Burst(8), Burst(0)),
            (_, Self::Neutral) => (Burst(8), Burst(8)),
            (_, Self::Regression) => (Burst(2), Burst(10)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Order {
    BaselineFirst,
    CandidateFirst,
}

pub(super) struct Spec {
    pub mutation: AcceptanceMutation,
    pub scenario: Scenario,
    pub pair: u32,
    pub order: Order,
    pub seconds: u64,
}

/// Two CPUs of the allowed set that are not SMT siblings of each other, so contention on one
/// does not leak onto the other through a shared core.
fn pick_cpus() -> anyhow::Result<(u32, u32)> {
    let status = fs::read_to_string("/proc/self/status")?;
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .context("no allowed CPU list")?;
    let mut cpus = Vec::new();
    for part in list.trim().split(',') {
        match part.split_once('-') {
            Some((low, high)) => cpus.extend(low.parse::<u32>()?..=high.parse::<u32>()?),
            None => cpus.push(part.parse()?),
        }
    }
    let siblings = |cpu: u32| -> Vec<u32> {
        fs::read_to_string(format!(
            "/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"
        ))
        .map(|text| {
            text.trim()
                .split(',')
                .flat_map(|part| match part.split_once('-') {
                    Some((low, high)) => {
                        (low.parse().unwrap_or(0)..=high.parse().unwrap_or(0)).collect::<Vec<_>>()
                    }
                    None => vec![part.parse().unwrap_or(u32::MAX)],
                })
                .collect()
        })
        .unwrap_or_default()
    };
    // Skip CPU 0 when possible: it commonly takes interrupts and housekeeping.
    let preferred: Vec<u32> = cpus.iter().copied().filter(|cpu| *cpu != 0).collect();
    let pool = if preferred.len() >= 2 {
        preferred
    } else {
        cpus
    };
    let first = *pool.first().context("no CPU available")?;
    let second = pool
        .iter()
        .copied()
        .find(|cpu| *cpu != first && !siblings(first).contains(cpu))
        .context("the allowed CPU set has no second core")?;
    Ok((first, second))
}

fn profile_toml(destination: u32) -> String {
    format!(
        "[[profile]]\nname = \"game-isolate-render-benefit\"\n\n[[profile.rules]]\naffinity = \"{destination}\"\nmatch_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n"
    )
}

/// The controller input for a CPU-affinity pair: the production defaults for everything the
/// experiment does not control.
fn controller_input(
    mutation: AcceptanceMutation,
    pid: u32,
    profiles: Option<PathBuf>,
    decision_log: PathBuf,
    window_seconds: u64,
) -> AutotuneCommandInput {
    let medium = mutation.mode() == stutter::api::daemon::DaemonMode::ApplyMediumRisk;
    AutotuneCommandInput {
        config: None,
        watch_process: None,
        tree_pid: Some(pid),
        profiles,
        mode: mutation.mode(),
        decision_log: Some(decision_log),
        duration_seconds: Some(window_seconds),
        washout_seconds: 2,
        washout_verify_interval_ms: 250,
        summary_ms: 250,
        // Scheduler evidence only: the benefit pair measures runnable latency, and the optional
        // probes add nothing to it (sched_stat_wait is not decodable on every kernel).
        preset: "lightweight".to_owned(),
        hwmon: false,
        mangohud_log: None,
        auto_focus: false,
        min_focus_confidence: stutter::api::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        focus_source: stutter::api::config::FocusSource::Hybrid,
        foreground_window: false,
        foreground_source: stutter::api::config::ForegroundSource::Auto,
        foreground_poll_ms: 1000,
        foreground_max_stale_ms: 2500,
        allow_system_wide_suggestions: false,
        allow_medium_risk: medium,
        high_risk_dry_run: false,
        dry_run_all_safe: false,
    }
}

/// The hidden controller process: runs the production live controller until it is signalled.
pub(super) fn run_controller(
    mutation: AcceptanceMutation,
    pid: u32,
    profiles: Option<&Path>,
    out: &Path,
    window_seconds: u64,
) -> anyhow::Result<()> {
    let grant = ValidationOverride {
        operation: match mutation {
            AcceptanceMutation::Affinity => MutationOperationKind::CpuAffinityProfile,
            AcceptanceMutation::Uclamp => MutationOperationKind::Uclamp,
            AcceptanceMutation::Nice => MutationOperationKind::Nice,
            AcceptanceMutation::Cgroup => MutationOperationKind::CgroupPlacement,
            AcceptanceMutation::CpuPowerEpp => MutationOperationKind::CpuPowerEpp,
        },
        objective: ObjectiveKind::GameRunnableLatency,
        campaign_id: "affinity-benefit".to_owned(),
    };
    autotune_command_blocking(
        controller_input(
            mutation,
            pid,
            profiles.map(Path::to_path_buf),
            out.join("decision.jsonl"),
            window_seconds,
        ),
        Some(ValidationRun {
            grant,
            history_log: out.join("history.jsonl"),
            controller_journal_path: out.join("controller.json"),
            session_seconds: window_seconds * 3 + 60,
            in_process_privileged: mutation.mode()
                == stutter::api::daemon::DaemonMode::ApplyMediumRisk,
            // The contention this pair started is the pair process's own children.
            offender_scope_parent: Some(std::os::unix::process::parent_id()),
            game_cgroup: (mutation == AcceptanceMutation::Cgroup)
                .then(|| delegated_target().ok())
                .flatten(),
        }),
    )
}

/// The `target` sibling of the leaf this process runs in, relative to the cgroup root.
fn delegated_target() -> anyhow::Result<PathBuf> {
    let text = fs::read_to_string("/proc/self/cgroup")?;
    let own = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("cgroup v2 path missing")?;
    let parent = Path::new(own.trim())
        .parent()
        .filter(|parent| parent.components().count() > 1)
        .context("this process does not run in a delegated cgroup subtree")?;
    Ok(parent.join("target"))
}

/// Creates the administrator's "game cgroup": a sibling of the leaf with its own CPU: the quiet
/// one. Moving the game there is the placement the provider proposes.
fn prepare_game_cgroup(quiet_cpu: u32) -> anyhow::Result<()> {
    let dir = Path::new("/sys/fs/cgroup").join(delegated_target()?.strip_prefix("/")?);
    fs::create_dir_all(&dir)?;
    let parent = dir.parent().context("target has no parent")?;
    fs::write(
        dir.join("cpuset.mems"),
        fs::read_to_string(parent.join("cpuset.mems.effective"))?,
    )?;
    fs::write(dir.join("cpuset.cpus"), quiet_cpu.to_string())?;
    Ok(())
}

/// Whether the controller's mutation is physically visible on any task of `pid`.
fn mutation_observed(
    mutation: AcceptanceMutation,
    pid: u32,
    destination: &str,
    contenders: &[u32],
) -> bool {
    match mutation {
        // The EPP benefit study is not a pair of this harness (it needs the privileged worker).
        AcceptanceMutation::CpuPowerEpp => false,
        AcceptanceMutation::Cgroup => fs::read_dir(format!("/proc/{pid}/task"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                fs::read_to_string(entry.path().join("cgroup"))
                    .is_ok_and(|text| text.trim_end().ends_with("/target"))
            }),
        // The demoted processes are the contenders, not the target.
        AcceptanceMutation::Nice => contenders.iter().any(|contender| {
            fs::read_to_string(format!("/proc/{contender}/stat"))
                .ok()
                .and_then(|stat| {
                    stat.rsplit_once(')')?
                        .1
                        .split_whitespace()
                        .nth(16)?
                        .parse::<i32>()
                        .ok()
                })
                .is_some_and(|nice| nice >= 10)
        }),
        AcceptanceMutation::Affinity => task_affinities(pid).iter().any(|a| a == destination),
        AcceptanceMutation::Uclamp => fs::read_dir(format!("/proc/{pid}/task"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                fs::read_to_string(entry.path().join("sched")).is_ok_and(|sched| {
                    sched.lines().any(|line| {
                        line.split_once(':').is_some_and(|(key, value)| {
                            key.trim() == "uclamp.min"
                                && value.trim().parse::<u32>().is_ok_and(|min| min > 0)
                        })
                    })
                })
            }),
    }
}

/// `Cpus_allowed_list` of every task of `pid`, to observe the placement the controller applied.
fn task_affinities(pid: u32) -> Vec<String> {
    fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            fs::read_to_string(entry.path().join("status"))
                .ok()?
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .map(|value| value.trim().to_owned())
        })
        .collect()
}

fn interrupt(child: &Child) {
    // SIGINT is the production controller's clean-shutdown request (it restores applied actions).
    let _ = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status();
}

/// The populations the objective compared, from the typed experiment evidence in the history.
fn experiment_from_history(history: &Path) -> Option<Value> {
    fs::read_to_string(history)
        .ok()?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| {
            event
                .get("objective_evidence")
                .is_some_and(|e| !e.is_null())
        })
}

fn population(signals: &Value) -> Option<Value> {
    Some(json!({
        "p99_ns": signals["runnable_latency_p99_ns"].as_u64()?,
        "samples": signals["runnable_samples"].as_u64()?,
        "over_5ms": signals["runnable_over_5ms"].as_u64()?,
    }))
}

pub(super) fn run(root: &Path, out: &Path, binary: &Path, spec: &Spec) -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "linux"), "the benefit pair requires Linux");
    if spec.mutation == AcceptanceMutation::Nice {
        // The contention processes inherit the limit, so the controller can un-demote them.
        stutter::api::autotune::prepare_nice_validation_process()?;
    }
    fs::create_dir(out).context("pair output must be a new directory with an existing parent")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(binary);
    let mut stage = "preflight";
    let result = (|| -> anyhow::Result<Value> {
        let (a, b) = pick_cpus()?;
        let (c0, c1) = match spec.order {
            Order::BaselineFirst => (a, b),
            Order::CandidateFirst => (b, a),
        };
        let game = game_runnable::install_game_binary(&out)?;
        if spec.mutation == AcceptanceMutation::Cgroup {
            prepare_game_cgroup(c1)?;
        }
        let profiles = (spec.mutation == AcceptanceMutation::Affinity).then(|| {
            let path = out.join("profiles.toml");
            fs::write(&path, profile_toml(c1)).map(|()| path)
        });
        let profiles = profiles.transpose()?;
        let total = (spec.seconds * 3 + 120).min(600);
        let (contention0, contention1) = spec.scenario.contention(spec.mutation);
        let mut contenders = game_runnable::spawn_contenders(c0, total, &out, contention0)?;
        contenders.extend(game_runnable::spawn_contenders(
            c1,
            total,
            &out,
            contention1,
        )?);
        let contender_pids: Vec<u32> = contenders.iter().map(|child| child.0.id()).collect();
        let ready = out.join("workload-ready.json");
        let mut target = OwnedChild(
            Command::new("taskset")
                .args(["-c", &c0.to_string()])
                .arg(&game)
                .args(["live-workload", "--seconds", &total.to_string(), "--ready"])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(fs::File::create(out.join("workload.stderr"))?)
                .unprivileged()
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            ensure!(
                target.0.try_wait()?.is_none(),
                "workload exited before readiness"
            );
            ensure!(Instant::now() < deadline, "workload readiness timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid = target.0.id();
        let ticks = start_ticks(pid)?;
        stage = "controller";
        let history = out.join("history.jsonl");
        let mut controller = OwnedChild(
            Command::new(std::env::current_exe()?)
                .args(["live-acceptance", "affinity-benefit-controller"])
                .args(["--pid", &pid.to_string(), "--window-seconds"])
                .arg(spec.seconds.to_string())
                .args(["--mutation", spec.mutation.as_str()])
                .args(
                    profiles
                        .iter()
                        .flat_map(|path| ["--profiles".as_ref(), path.as_os_str()]),
                )
                .arg("--out")
                .arg(&out)
                .arg("--stutter-bin")
                .arg(&binary)
                .stdout(fs::File::create(out.join("controller.stdout"))?)
                .stderr(fs::File::create(out.join("controller.stderr"))?)
                .spawn()?,
        );
        // The experiment is one baseline window, a washout and one candidate window; allow for
        // the data-quality gate before the baseline is accepted.
        let deadline = Instant::now() + Duration::from_secs(spec.seconds * 3 + 60);
        let destination = c1.to_string();
        let mut placement_observed = false;
        let mut event = None;
        while Instant::now() < deadline {
            if let Some(status) = controller.0.try_wait()? {
                anyhow::bail!(
                    "the controller exited early ({}); inspect controller.stderr",
                    status
                        .signal()
                        .map_or_else(|| status.to_string(), |s| format!("signal {s}"))
                );
            }
            ensure!(
                target.0.try_wait()?.is_none(),
                "the target exited during the experiment"
            );
            placement_observed |=
                mutation_observed(spec.mutation, pid, &destination, &contender_pids);
            if let Some(found) = experiment_from_history(&history) {
                event = Some(found);
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        interrupt(&controller.0);
        let stop_deadline = Instant::now() + Duration::from_secs(15);
        while controller.0.try_wait()?.is_none() && Instant::now() < stop_deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(controller);
        let event = event.context("the controller recorded no keep/revert comparison in time")?;
        ensure!(
            start_ticks(pid)? == ticks,
            "target identity changed during the experiment"
        );
        let evidence = &event["objective_evidence"];
        let baseline = population(&evidence["baseline_signals"])
            .context("the baseline runnable-latency population is incomplete")?;
        let candidate = population(&evidence["candidate_signals"])
            .context("the candidate runnable-latency population is incomplete")?;
        let kept = event["rollback_performed"] == false;
        let guard_failures = evidence["decision"]["guard_failures"]
            .as_array()
            .map_or(0, Vec::len);
        drop(target);
        drop(contenders);
        Ok(json!({
            "scenario": spec.scenario.name(),
            "pair": spec.pair,
            "order": match spec.order { Order::BaselineFirst => "BaselineFirst", Order::CandidateFirst => "CandidateFirst" },
            "baseline": baseline,
            "candidate": candidate,
            "placement_observed": placement_observed,
            "guardrails_ok": guard_failures == 0,
            "manager_kept": kept,
            "cpus": {"baseline": c0, "destination": c1},
            "objective_outcome": evidence["decision"]["outcome"],
        }))
    })();
    let status = if result.is_ok() { "PASS" } else { "FAIL" };
    let body = json!({
        "schema_version": 1,
        "status": status,
        "stage": stage,
        "evidence_kind": "controlled_real",
        "pair": result.as_ref().ok(),
        "error": result.as_ref().err().map(|err| format!("{err:#}")),
        "mutation": spec.mutation.as_str(),
        "scope": format!("one matched pair of the {} benefit experiment through the production live controller", spec.mutation.as_str()),
    });
    fs::write(out.join("result.json"), serde_json::to_vec_pretty(&body)?)?;
    if let Ok(pair) = &result {
        fs::write(out.join("pair.json"), serde_json::to_vec_pretty(pair)?)?;
    }
    println!("{status}: benefit pair {}", out.display());
    result.map(|_| ())
}
