//! One round of the soak: a workload on a contended CPU, the real controller on it, and the
//! scheduled perturbation. A round never asserts by panicking: whatever goes wrong is written to
//! its record, so a violation is evidence rather than the end of the soak.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::Context;

use super::{
    history::{HistoryEvent, Tail, audit_events},
    plan::{Kind, Operations, Round, Timing},
    records::{EventRecord, RoundRecord, SessionRecord, unix_ms},
    world::{Controller, Game, World, signal, wait_exit},
};

/// Set by SIGINT/SIGTERM on the soak process: finish nothing new, record the round as interrupted.
pub(super) static STOP: AtomicBool = AtomicBool::new(false);

const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

struct Ctx<'a> {
    world: &'a mut World,
    dir: PathBuf,
    timing: Timing,
    round: Round,
    record: RoundRecord,
    tail: Tail,
    /// Events polled but not yet claimed by a wait; waits take the first match, in order.
    pending: Vec<HistoryEvent>,
    /// Every decision the controller made during the round, for the schedule's bookkeeping.
    seen: Vec<HistoryEvent>,
    /// Experiments whose resolution a wait already claimed (the runtime repeats its context).
    resolved: Vec<Option<String>>,
}

pub(super) fn run_round(
    world: &mut World,
    dir: &Path,
    index: u32,
    round: Round,
    timing: Timing,
) -> (RoundRecord, Vec<HistoryEvent>) {
    let dir = dir.join(format!("rounds/{index:04}"));
    let mut ctx = Ctx {
        tail: Tail::at_end(&world.history_path()),
        pending: Vec::new(),
        seen: Vec::new(),
        resolved: Vec::new(),
        record: RoundRecord {
            round: index,
            kind: round.kind.name().to_owned(),
            scenario: round.scenario.name().to_owned(),
            operations: round.operations.name().to_owned(),
            started_unix_ms: unix_ms(),
            ..RoundRecord::default()
        },
        world,
        dir,
        timing,
        round,
    };
    let result = fs::create_dir_all(&ctx.dir)
        .map_err(anyhow::Error::from)
        .and_then(|()| ctx.execute());
    ctx.record.finished_unix_ms = unix_ms();
    match result {
        Ok(()) if ctx.record.violations.is_empty() => ctx.record.outcome = "ok".to_owned(),
        Ok(()) => ctx.record.outcome = "failed".to_owned(),
        Err(error) => {
            ctx.record.outcome = if STOP.load(Ordering::SeqCst) {
                "interrupted"
            } else {
                "failed"
            }
            .to_owned();
            ctx.record.error = Some(format!("{error:#}"));
        }
    }
    ctx.poll_events();
    (ctx.record, ctx.seen)
}

impl Ctx<'_> {
    fn event(&mut self, kind: &str, detail: impl Into<String>) {
        self.record.events.push(EventRecord {
            unix_ms: unix_ms(),
            kind: kind.to_owned(),
            detail: detail.into(),
        });
    }

    fn violation(&mut self, message: impl Into<String>) {
        self.record.violations.push(message.into());
    }

    fn execute(&mut self) -> anyhow::Result<()> {
        if self.world.ensure_worker()? {
            self.event("worker_start", "privileged worker started");
        }
        self.world.prepare_game_cgroup()?;
        let (burst0, burst1) = self.round.scenario.bursts();
        let seconds = self.timing.long_round_s + self.timing.resolve_timeout_s * 3 + 120;
        let (c0, c1) = self.world.cpus;
        let _contention = (
            self.world.spawn_burst(c0, burst0, seconds)?,
            self.world.spawn_burst(c1, burst1, seconds)?,
        );
        let kind = self.round.kind;
        // `live-workload` is bounded to ten minutes.
        let game_seconds = if kind == Kind::Long {
            self.timing.long_round_s
        } else {
            seconds.min(600)
        };
        let game = self.world.spawn_game(&self.dir, "game-1", game_seconds)?;
        self.event("game_start", format!("pid {}", game.child.pid()));
        let outcome = self.drive(game);
        // Whatever happened, leave nothing of this round running or applied.
        self.settle();
        outcome
    }

    fn drive(&mut self, mut game: Game) -> anyhow::Result<()> {
        // A focus swap hands the next workload to the other graduated operation: the first one's
        // candidate memory rightly holds it back for five minutes.
        let first = if self.round.kind == Kind::FocusSwap {
            Operations::Affinity
        } else {
            self.round.operations
        };
        let mut controller = self.start_controller(&game, "controller-1", first)?;
        match self.round.kind {
            Kind::Plain => {
                self.experiment(&mut controller, &game)?;
                self.end_workload(&mut game, controller, "workload_exit")
            }
            Kind::ControllerCrash => {
                let tried = self.applied(&mut controller)?;
                self.kill_controller(&mut controller);
                self.end_session(controller, "kill9");
                let mut next =
                    self.start_controller(&game, "controller-2", self.round.operations)?;
                self.wait_recovery()?;
                self.memory_holds(&mut next, tried)?;
                self.end_workload(&mut game, next, "workload_exit")
            }
            Kind::ControllerStop => {
                let tried = self.applied(&mut controller)?;
                self.stop_controller(&mut controller);
                self.end_session(controller, "sigint");
                self.check_restored(&game);
                let mut next =
                    self.start_controller(&game, "controller-2", self.round.operations)?;
                self.memory_holds(&mut next, tried)?;
                self.end_workload(&mut game, next, "workload_exit")
            }
            Kind::WorkerKill => {
                self.applied(&mut controller)?;
                self.world.kill_worker();
                self.event("worker_kill", "kill -9 of the privileged worker");
                std::thread::sleep(Duration::from_secs(3));
                if self.world.ensure_worker()? {
                    self.event("worker_start", "privileged worker restarted");
                }
                if self.resolution(&mut controller)?.is_none()
                    && controller.child.0.try_wait()?.is_some()
                {
                    // The controller faulted with the worker; the supervisor starts another.
                    self.end_session(controller, "faulted_with_worker");
                    controller =
                        self.start_controller(&game, "controller-2", self.round.operations)?;
                    self.experiment(&mut controller, &game)?;
                }
                self.end_workload(&mut game, controller, "workload_exit")
            }
            Kind::Suspend => {
                self.applied(&mut controller)?;
                self.suspend()?;
                if self.resolution(&mut controller)?.is_none() {
                    self.violation("no experiment resolution after the machine resumed");
                }
                self.end_workload(&mut game, controller, "workload_exit")
            }
            Kind::WorkloadKill => {
                self.applied(&mut controller)?;
                signal(game.child.pid(), 9);
                self.event("game_killed", "kill -9 of the workload mid-experiment");
                let _ = wait_exit(&mut game.child, Duration::from_secs(10));
                let status = wait_exit(&mut controller.child, Duration::from_secs(60))?;
                self.end_session(
                    controller,
                    if status.is_some() {
                        "workload_killed"
                    } else {
                        "stuck"
                    },
                );
                Ok(())
            }
            Kind::FocusSwap => {
                self.experiment(&mut controller, &game)?;
                self.end_workload(&mut game, controller, "workload_exit")?;
                let second = self.world.spawn_game(
                    &self.dir,
                    "game-2",
                    self.timing.resolve_timeout_s * 2 + 60,
                )?;
                self.event("game_start", format!("pid {}", second.child.pid()));
                let mut next =
                    self.start_controller(&second, "controller-2", Operations::Cgroup)?;
                self.experiment(&mut next, &second)?;
                let mut second = second;
                self.end_workload(&mut second, next, "workload_exit")
            }
            Kind::Long => {
                let deadline = Instant::now() + Duration::from_secs(self.timing.long_round_s);
                let mut reaped = false;
                while Instant::now() < deadline && !STOP.load(Ordering::SeqCst) {
                    if controller.child.0.try_wait()?.is_some() {
                        break;
                    }
                    // The workload ends on its own at the end of the round; this process is its
                    // launcher and must reap it, or the zombie still looks alive to the controller.
                    if game.child.0.try_wait()?.is_some() {
                        reaped = true;
                        break;
                    }
                    self.poll_events();
                    std::thread::sleep(Duration::from_millis(500));
                }
                if !reaped {
                    return self.end_workload(&mut game, controller, "workload_exit");
                }
                self.event("game_exit", "the workload ran its course");
                let status = wait_exit(&mut controller.child, Duration::from_secs(60))?;
                self.end_session(
                    controller,
                    if status.is_some() {
                        "workload_exit"
                    } else {
                        "stuck_after_workload_exit"
                    },
                );
                Ok(())
            }
        }
    }

    fn start_controller(
        &mut self,
        game: &Game,
        name: &str,
        operations: Operations,
    ) -> anyhow::Result<Controller> {
        let controller = self.world.spawn_controller(
            &self.dir,
            name,
            game.child.pid(),
            operations,
            self.timing,
        )?;
        self.event("controller_start", name);
        Ok(controller)
    }

    fn poll_events(&mut self) {
        let fresh = self.tail.poll();
        self.seen.extend(fresh.iter().cloned());
        self.pending.extend(fresh);
    }

    fn take_matching(&mut self, matches: impl Fn(&HistoryEvent) -> bool) -> Option<HistoryEvent> {
        self.poll_events();
        let at = self.pending.iter().position(matches)?;
        Some(self.pending.remove(at))
    }

    /// Poll the history until `matches` (or the timeout, or the controller exiting).
    fn wait_for(
        &mut self,
        controller: &mut Controller,
        timeout: Duration,
        matches: impl Fn(&HistoryEvent) -> bool,
    ) -> anyhow::Result<Option<HistoryEvent>> {
        let deadline = Instant::now() + timeout;
        loop {
            anyhow::ensure!(!STOP.load(Ordering::SeqCst), "the soak was interrupted");
            if let Some(event) = self.take_matching(&matches) {
                return Ok(Some(event));
            }
            if controller.child.0.try_wait()?.is_some() || Instant::now() >= deadline {
                return Ok(self.take_matching(&matches));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Wait for an experiment to start; its action id, when it did.
    fn applied(&mut self, controller: &mut Controller) -> anyhow::Result<Option<String>> {
        let timeout = Duration::from_secs(self.timing.resolve_timeout_s);
        let event = self.wait_for(controller, timeout, |e| e.decision == "candidate_applied")?;
        match event {
            Some(event) => {
                self.event(
                    "candidate_applied",
                    event.action_kind.clone().unwrap_or_default(),
                );
                Ok(event.action_id)
            }
            None => {
                self.violation("no experiment started in time");
                Ok(None)
            }
        }
    }

    /// A restarted controller remembers what its predecessor tried: the same action is not tried
    /// again at once (candidate memory persisted across the restart).
    fn memory_holds(
        &mut self,
        controller: &mut Controller,
        tried: Option<String>,
    ) -> anyhow::Result<()> {
        let window = Duration::from_secs(self.timing.candidate_window_s + 20);
        let again = self.wait_for(controller, window, |e| {
            e.decision == "candidate_applied" && tried.is_some() && e.action_id == tried
        })?;
        if again.is_some() {
            self.violation(
                "the restarted controller retried the same action at once (candidate memory lost)",
            );
        } else {
            self.event(
                "memory_holds",
                "no immediate retry of the action the crashed controller tried",
            );
        }
        Ok(())
    }

    fn resolution(&mut self, controller: &mut Controller) -> anyhow::Result<Option<HistoryEvent>> {
        let timeout = Duration::from_secs(self.timing.resolve_timeout_s);
        let resolved = self.resolved.clone();
        let event = self.wait_for(controller, timeout, move |event| {
            event.is_resolution() && !resolved.contains(&event.experiment_id)
        })?;
        if let Some(event) = &event {
            self.resolved.push(event.experiment_id.clone());
        }
        if let Some(event) = &event {
            self.event(
                &event.decision,
                event.action_kind.clone().unwrap_or_default(),
            );
        }
        Ok(event)
    }

    fn experiment(&mut self, controller: &mut Controller, _game: &Game) -> anyhow::Result<()> {
        self.applied(controller)?;
        if self.resolution(controller)?.is_none() {
            self.violation("an experiment did not resolve in time");
        }
        std::thread::sleep(Duration::from_secs(5));
        Ok(())
    }

    fn kill_controller(&mut self, controller: &mut Controller) {
        signal(controller.child.pid(), 9);
        self.event(
            "controller_kill9",
            "kill -9 of the controller mid-experiment",
        );
    }

    fn stop_controller(&mut self, controller: &mut Controller) {
        signal(controller.child.pid(), SIGINT);
        self.event(
            "controller_sigint",
            "SIGINT of the controller mid-experiment",
        );
    }

    /// Record how a controller session ended (waiting for it to be gone first).
    fn end_session(&mut self, mut controller: Controller, ended_by: &str) {
        let status = wait_exit(&mut controller.child, Duration::from_secs(45))
            .ok()
            .flatten();
        if status.is_none() {
            signal(controller.child.pid(), 9);
            self.violation(format!("controller session {ended_by} did not exit in 45s"));
            let _ = wait_exit(&mut controller.child, Duration::from_secs(10));
        }
        use std::os::unix::process::ExitStatusExt;
        self.record.sessions.push(SessionRecord {
            start_unix_ms: controller.started_unix_ms,
            end_unix_ms: unix_ms(),
            exit_code: status.and_then(|s| s.code()),
            signal: status.and_then(|s| s.signal()),
            ended_by: ended_by.to_owned(),
        });
    }

    /// The workload leaves normally; the controller must follow it and leave nothing applied.
    fn end_workload(
        &mut self,
        game: &mut Game,
        mut controller: Controller,
        ended_by: &str,
    ) -> anyhow::Result<()> {
        signal(game.child.pid(), SIGTERM);
        self.event("game_exit", "SIGTERM of the workload");
        // A launcher reaps its game; until this process does, the exited game is a zombie and
        // still looks alive to the controller.
        let _ = wait_exit(&mut game.child, Duration::from_secs(10));
        let status = wait_exit(&mut controller.child, Duration::from_secs(60))?;
        let how = if status.is_some() {
            ended_by
        } else {
            "stuck_after_workload_exit"
        };
        self.end_session(controller, how);
        Ok(())
    }

    fn wait_recovery(&mut self) -> anyhow::Result<()> {
        let before = audit_events(&self.world.audit_path(), "autotune-startup-recovery").len();
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            let events = audit_events(&self.world.audit_path(), "autotune-startup-recovery");
            if events.len() > before {
                if events.iter().skip(before).all(|(ok, _)| *ok) {
                    self.event(
                        "startup_recovery",
                        "the restarted controller restored the crashed one's action",
                    );
                } else {
                    self.violation("startup recovery reported a failed restore");
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.violation("the restarted controller did not run startup recovery");
                return Ok(());
            }
            anyhow::ensure!(!STOP.load(Ordering::SeqCst), "the soak was interrupted");
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn suspend(&mut self) -> anyhow::Result<()> {
        let stats = || {
            fs::read_to_string("/sys/power/suspend_stats/success")
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
        };
        let before = stats();
        self.event("suspend", format!("rtcwake for {}s", self.timing.suspend_s));
        let status = std::process::Command::new("sudo")
            .args([
                "-n",
                "rtcwake",
                "-m",
                "mem",
                "-s",
                &self.timing.suspend_s.to_string(),
            ])
            .status()
            .context("run rtcwake")?;
        self.event("resume", format!("rtcwake exited {status}"));
        if !status.success() || stats() <= before {
            self.violation("the machine did not complete a suspend/resume cycle");
        }
        Ok(())
    }

    /// After a controller stopped while the workload lives on, nothing may still be applied.
    fn check_restored(&mut self, game: &Game) {
        if super::world::start_ticks(game.child.pid()).ok() != Some(game.ticks) {
            self.violation("the workload process changed identity while the controller stopped");
        }
        let allowed = task_allowed_lists(game.child.pid());
        let home = self.world.cpus.0.to_string();
        if allowed.iter().any(|list| *list != home) {
            self.violation(format!(
                "after the controller stopped the workload runs on {allowed:?}, not {home}"
            ));
        }
        self.check_quiescent(true);
    }

    /// Durable state at rest: the journal names no live token, the ledger is gone and the game
    /// cgroup is empty.
    fn check_quiescent(&mut self, workload_alive: bool) {
        if let Ok(text) = fs::read_to_string(self.world.journal_path()) {
            {
                let state = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v["state"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unreadable".to_owned());
                if !matches!(state.as_str(), "clean" | "reverted") {
                    self.violation(format!("the controller journal is `{state}` at rest"));
                }
            }
        }
        if self.world.ledger_path().exists() {
            self.violation("the kept-action ledger still holds an entry at rest");
        }
        if !workload_alive && !self.world.game_cgroup_procs().is_empty() {
            self.violation("the game cgroup still holds tasks after the workload ended");
        }
    }

    /// End of round: stop everything this round started and check what remains.
    fn settle(&mut self) {
        std::thread::sleep(Duration::from_secs(1));
        self.check_quiescent(false);
    }
}

/// `Cpus_allowed_list` of every task of `pid`.
pub(super) fn task_allowed_lists(pid: u32) -> Vec<String> {
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
