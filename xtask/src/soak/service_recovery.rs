//! Live recovery acceptance through the installed service: every lifecycle and fault event an
//! administrator or the machine can inflict, with root-token and physical-restoration readback.
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter_validation_runner::model::{
    BoundedServiceAction as Action, ServiceComponent, ServiceSignal,
};

use super::{
    history::Tail,
    plan::{Operations, timing},
    records::Profile,
    service::Binding,
    service_probe::{Probe, focus_matches, stop_game},
    world::{Controller, Game, World},
};
use crate::service_campaign::model::{Stage, State};

struct Lab<'a> {
    state: &'a State,
    dir: &'a Path,
    binding: Binding,
    world: World,
    probe: Probe,
    cpus: (u32, u32),
    legs: Vec<Value>,
}

fn new_lab<'a>(state: &'a State, dir: &'a Path, stage: Stage) -> anyhow::Result<Lab<'a>> {
    fs::create_dir_all(dir)?;
    let cpus = super::pick_cpus()?;
    let binding = Binding::new(state, stage)?;
    let mut world = World::new(&state.repository, dir, &state.pinned_stutter(), cpus)?;
    world.bind_service(binding.clone())?;
    world.prepare_round(Operations::Both)?;
    let probe = Probe {
        tail: Tail::at_end(&world.history_path()),
        records: vec![],
        owned: Default::default(),
    };
    Ok(Lab {
        state,
        dir,
        binding,
        world,
        probe,
        cpus,
        legs: Vec::new(),
    })
}

/// Brings the service to a state with an applied action and a root-issued token outstanding, then
/// leaves the workload, contention and service running for the reboot that follows. Nothing here
/// is cleaned up on purpose: the reboot is the event under test.
pub(crate) fn applied_before_reboot(state: &State, dir: &Path) -> anyhow::Result<Value> {
    let mut lab = new_lab(state, dir, Stage::Reboots)?;
    let mut controller = lab.controller("reboot-pre")?;
    let (game, pressure) = lab.applied(&mut controller, "reboot-game")?;
    let report = lab.query()?;
    anyhow::ensure!(
        report["worker_tokens"]["outstanding"].as_u64() >= Some(1),
        "no root-issued token is outstanding to carry across the reboot: {report}"
    );
    let receipt = json!({
        "boot_id": report["boot_id"],
        "outstanding": report["worker_tokens"]["outstanding"],
        "recovery_audit_completed": report["worker_tokens"]["recovery_audit"]["completed"],
        "controller_pid": report["controller_process"]["pid"],
        "worker_pid": report["worker_process"]["pid"],
    });
    std::mem::forget(game);
    std::mem::forget(pressure);
    std::mem::forget(controller);
    Ok(receipt)
}

pub(crate) fn run(state: &State, dir: &Path) -> anyhow::Result<Value> {
    let mut lab = new_lab(state, dir, Stage::Recovery)?;
    let binding = lab.binding.clone();
    let result = lab.legs();
    let stopped = binding.action(Action::Stop);
    if let Err(error) = &result {
        fs::write(
            dir.join("failed-recovery.json"),
            serde_json::to_vec_pretty(
                &json!({"error":format!("{error:#}"),"legs":lab.legs,"history":lab.probe.records}),
            )?,
        )?;
    }
    stopped?;
    result
}

impl Lab<'_> {
    fn query(&self) -> anyhow::Result<Value> {
        self.binding.action(Action::Query)
    }

    fn state_of(report: &Value, unit: &str) -> String {
        report[unit]
            .as_str()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("ActiveState="))
            })
            .unwrap_or("unknown")
            .to_owned()
    }

    fn controller(&mut self, name: &str) -> anyhow::Result<Controller> {
        self.world.spawn_controller(
            self.dir,
            name,
            0,
            Operations::Both,
            timing(Profile::Service),
        )
    }

    /// A game under pressure whose experiment has been applied, so a fault lands mid-action.
    fn applied(
        &mut self,
        controller: &mut Controller,
        name: &str,
    ) -> anyhow::Result<(Game, Option<super::actors::Contention>)> {
        let game = self.world.spawn_game(self.dir, name, 590)?;
        self.probe.own(game.child.pid());
        // The controller may apply in the same poll batch that first shows the focus, so the
        // application is searched for from before the focus wait.
        let since = self.probe.mark();
        self.probe
            .wait_from(controller, since, 30, |row| focus_matches(row, &game))?;
        let pressure = self.world.spawn_burst(self.cpus.0, 8, 590)?;
        // The candidate cooldown of an earlier leg may still be running.
        self.probe.wait_from(controller, since, 420, |row| {
            row["decision"]["decision"] == "candidate_applied" && focus_matches(row, &game)
        })?;
        Ok((game, pressure))
    }

    /// Waits until the unit pair is running again with a controller other than `old_pid`.
    fn restored(&self, old_pid: u32) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let report = self.query()?;
            if super::service::attest(&report, self.state).is_ok()
                && report["controller_process"]["pid"] != old_pid
            {
                return Ok(report);
            }
            ensure!(
                Instant::now() < deadline,
                "service pair did not come back with a new controller: {report}"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Waits for the root ledger to be empty. A restarted controller may legitimately start the
    /// other operation on the still-running game, and an experiment takes a measurement window
    /// and a resolution to finish, so the budget covers one whole experiment.
    fn settled(&self) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let report = self.query()?;
            if report["worker_tokens"]["readable"] == true
                && report["worker_tokens"]["outstanding"] == 0
                && !self.world.ledger_path().exists()
            {
                return Ok(report);
            }
            ensure!(
                Instant::now() < deadline,
                "root tokens or the task ledger were not settled: {report}"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn recoveries(&self) -> anyhow::Result<u64> {
        self.query()?["worker_tokens"]["recovery_audit"]["completed"]
            .as_u64()
            .context("recovery audit count missing")
    }

    fn signal(&self, component: ServiceComponent, signal: ServiceSignal) -> anyhow::Result<()> {
        self.binding
            .action(Action::Signal { component, signal })
            .map(drop)
    }

    fn finish_leg(&mut self, name: &str, detail: Value) {
        self.legs.push(json!({"leg":name,"detail":detail}));
    }

    /// Release the workload and prove the always-on controller settled on its own.
    fn release(
        &mut self,
        controller: &mut Controller,
        mut game: Game,
        pressure: Option<super::actors::Contention>,
    ) -> anyhow::Result<Value> {
        drop(pressure);
        stop_game(&mut game)?;
        super::service_idle::wait(&self.world, controller)
    }

    /// The campaign's delegated cgroup subtree: the desktop user owns it and may create siblings.
    fn subtree(&self) -> std::path::PathBuf {
        Path::new("/sys/fs/cgroup").join(format!("stutter-validation-{}", self.state.campaign_id))
    }

    fn allowed_cpus(pid: u32) -> anyhow::Result<String> {
        fs::read_to_string(format!("/proc/{pid}/status"))?
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
            .map(|list| list.trim().to_owned())
            .context("allowed CPU list missing")
    }

    fn superseded_total(&self) -> anyhow::Result<u64> {
        Ok(
            self.query()?["worker_tokens"]["recovery_audit"]["dispositions"]
                ["superseded_externally"]
                .as_u64()
                .unwrap_or(0),
        )
    }

    /// A running controller meets an external owner and must understand it: it names the
    /// conflict, never applies again while it stands, and the root ledger ends with the token
    /// settled as *externally superseded* (not as restored, and not left open).
    fn observe_owner_conflict(&mut self, superseded_before: u64) -> anyhow::Result<Value> {
        let start = self.probe.records.len();
        let deadline = Instant::now() + Duration::from_secs(90);
        let detected = loop {
            self.probe.poll();
            if let Some(row) = self.probe.records[start..].iter().find(|row| {
                row["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("external_mutation_detected"))
            }) {
                break row.clone();
            }
            ensure!(
                Instant::now() < deadline,
                "the controller never detected the external owner (no external_mutation_detected)"
            );
            std::thread::sleep(Duration::from_millis(500));
        };
        let settle_by = Instant::now() + Duration::from_secs(60);
        let report = loop {
            let report = self.query()?;
            let superseded =
                report["worker_tokens"]["recovery_audit"]["dispositions"]["superseded_externally"]
                    .as_u64()
                    .unwrap_or(0);
            if report["worker_tokens"]["outstanding"] == 0 && superseded > superseded_before {
                break report;
            }
            ensure!(
                Instant::now() < settle_by,
                "the external takeover was not settled as superseded: {report}"
            );
            std::thread::sleep(Duration::from_millis(500));
        };
        self.probe.poll();
        ensure!(
            self.probe.records[start..]
                .iter()
                .all(|row| row["decision"]["decision"] != "candidate_applied"),
            "the controller applied again while an external owner held the task"
        );
        Ok(json!({
            "detected": detected["reason"],
            "decision": detected["decision"]["decision"],
            "dispositions": report["worker_tokens"]["recovery_audit"]["dispositions"],
        }))
    }

    /// Reconfigure the fixture for one operation family; both service actors are stopped first.
    fn reconfigure(&mut self, operations: Operations) -> anyhow::Result<()> {
        self.world.prepare_round(operations)
    }

    /// The root worker settled the token the interrupted controller left, and left nothing
    /// unresolved. The restarted controller may already hold a token of its own for a new
    /// experiment on the still-running game, so an empty ledger is not the invariant here; that
    /// is checked once the game is gone.
    fn recovered_after(&self, audits: u64, unresolved: u64) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let report = self.query()?;
            let audit = &report["worker_tokens"]["recovery_audit"];
            ensure!(
                audit["dispositions"]["unresolved"].as_u64().unwrap_or(0) == unresolved,
                "the restart left an unresolved token: {report}"
            );
            if report["worker_tokens"]["readable"] == true
                && audit["completed"].as_u64().unwrap_or(0) > audits
            {
                return Ok(report);
            }
            ensure!(
                Instant::now() < deadline,
                "no root recovery followed the interruption: {report}"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn unresolved_total(&self) -> anyhow::Result<u64> {
        Ok(
            self.query()?["worker_tokens"]["recovery_audit"]["dispositions"]["unresolved"]
                .as_u64()
                .unwrap_or(0),
        )
    }

    /// Stop and restart the controller from outside while its action is applied, then wait for
    /// the root recovery of the interrupted token.
    fn term_and_return(&mut self, controller: &mut Controller, name: &str) -> anyhow::Result<()> {
        let audits = self.recoveries()?;
        let unresolved = self.unresolved_total()?;
        let before = controller.child.pid();
        self.signal(ServiceComponent::Controller, ServiceSignal::Term)?;
        self.await_gone(before)?;
        *controller = self.controller(name)?;
        self.recovered_after(audits, unresolved)?;
        Ok(())
    }

    fn legs(&mut self) -> anyhow::Result<Value> {
        let mut controller = self.controller("recovery-0")?;
        let original = controller.child.pid();

        // An administrator stops only the controller: it must stay stopped, with the worker up.
        self.binding.action(Action::StopController)?;
        std::thread::sleep(Duration::from_secs(15));
        let report = self.query()?;
        ensure!(
            Self::state_of(&report, "controller") == "inactive"
                && Self::state_of(&report, "worker") == "active",
            "an administrative controller stop was undone or took the worker down: {report}"
        );
        controller = self.controller("recovery-1")?;
        ensure!(
            controller.child.pid() != original,
            "controller was not started again"
        );
        self.finish_leg(
            "administrative_controller_stop",
            json!({"stayed_stopped_seconds":15}),
        );

        // The root worker is killed while idle: the pair must return and the ledger be readable.
        let before = controller.child.pid();
        self.signal(ServiceComponent::Worker, ServiceSignal::Kill)?;
        self.restored(before)?;
        let recovered = self.settled()?;
        controller = self.controller("recovery-2")?;
        self.finish_leg(
            "idle_worker_kill",
            json!({"report":recovered,"new_controller":controller.child.pid()}),
        );

        // SIGTERM lands while a candidate is applied: graceful stop, physical restoration.
        let audits = self.recoveries()?;
        let (game, pressure) = self.applied(&mut controller, "term-game")?;
        self.term_and_return(&mut controller, "recovery-term")?;
        ensure!(
            self.recoveries()? > audits,
            "no new root recovery audit followed the SIGTERM"
        );
        let idle = self.release(&mut controller, game, pressure)?;
        self.finish_leg("sigterm_while_applied", json!({"idle":idle}));

        // A service restart while applied.
        let audits = self.recoveries()?;
        let (game, pressure) = self.applied(&mut controller, "restart-game")?;
        let unresolved = self.unresolved_total()?;
        let before = controller.child.pid();
        self.binding.action(Action::Restart)?;
        self.await_gone(before)?;
        controller = self.controller("recovery-restart")?;
        self.recovered_after(audits, unresolved)?;
        ensure!(
            self.recoveries()? > audits,
            "no new root recovery audit followed the service restart"
        );
        let idle = self.release(&mut controller, game, pressure)?;
        self.finish_leg("service_restart_while_applied", json!({"idle":idle}));

        // SIGKILL, then a corrupted controller journal. Safety must not depend on anyone
        // restarting the worker: the controller settles the worker's own ledger before it reads
        // the journal, then stays failed and applies nothing until the file is repaired.
        let audits = self.recoveries()?;
        let (game, pressure) = self.applied(&mut controller, "corrupt-game")?;
        let applied_before = self.probe.records.len();
        let before = controller.child.pid();
        self.signal(ServiceComponent::Controller, ServiceSignal::Kill)?;
        self.await_gone(before)?;
        let journal = self.world.journal_path();
        fs::write(&journal, b"{ this journal was corrupted by the acceptance")?;
        let settled = self.settled()?;
        ensure!(
            self.recoveries()? > audits,
            "the worker did not settle its own ledger for a controller that cannot read its journal"
        );
        self.probe.poll();
        ensure!(
            self.probe.records[applied_before..]
                .iter()
                .all(|row| row["decision"]["decision"] != "candidate_applied"),
            "a controller with a corrupt journal applied an action"
        );
        let report = self.query()?;
        ensure!(
            report["controller_process"]["consistent"] != true,
            "a controller kept running on a corrupt journal: {report}"
        );
        let cgroup = fs::read_to_string(format!("/proc/{}/cgroup", game.child.pid()))?;
        ensure!(
            !cgroup.trim().ends_with("/target"),
            "the task was left in the experiment cgroup: {cgroup}"
        );
        fs::rename(&journal, journal.with_extension("json.corrupt"))?;
        controller = self.controller("recovery-corrupt")?;
        let idle = self.release(&mut controller, game, pressure)?;
        self.finish_leg(
            "sigkill_with_corrupt_journal",
            json!({"settled":settled,"idle":idle}),
        );

        // Someone else sets the affinity to a value that is neither the original (one CPU) nor
        // the one Stutter applied (the other): it must survive Stutter's rollback.
        self.reconfigure(Operations::Affinity)?;
        controller = self.controller("recovery-affinity")?;
        let (game, pressure) = self.applied(&mut controller, "external-affinity")?;
        let superseded_before = self.superseded_total()?;
        let both = format!("{},{}", self.cpus.0, self.cpus.1);
        let output = Command::new("taskset")
            .args(["-a", "-p", "-c", &both])
            .arg(game.child.pid().to_string())
            .output()
            .context("run taskset")?;
        ensure!(output.status.success(), "the external mutation failed");
        let external = Self::allowed_cpus(game.child.pid())?;
        let affinity_conflict = self.observe_owner_conflict(superseded_before)?;
        ensure!(
            controller.child.try_wait()?.is_none() || self.query().is_ok(),
            "an external affinity change took the service away"
        );
        let victim = controller.child.pid();
        if controller.child.try_wait()?.is_none() {
            self.term_and_return(&mut controller, "recovery-affinity-2")?;
        } else {
            self.restored(victim)?;
            controller = self.controller("recovery-affinity-2")?;
            self.settled()?;
        }
        let after = Self::allowed_cpus(game.child.pid())?;
        ensure!(
            after == external,
            "Stutter overwrote an external affinity: was {external}, now {after}"
        );
        let idle = self.release(&mut controller, game, pressure)?;
        self.finish_leg(
            "external_affinity_owner_preserved",
            json!({"external":external,"after":after,"idle":idle,"autonomous_handling":affinity_conflict}),
        );

        // The same for cgroup placement: a sibling cgroup of the campaign's own.
        self.reconfigure(Operations::Cgroup)?;
        controller = self.controller("recovery-cgroup")?;
        let (game, pressure) = self.applied(&mut controller, "external-cgroup")?;
        let superseded_before = self.superseded_total()?;
        let sibling = self.subtree().join("external");
        fs::create_dir_all(&sibling)?;
        fs::write(sibling.join("cgroup.procs"), game.child.pid().to_string())?;
        let placed = fs::read_to_string(format!("/proc/{}/cgroup", game.child.pid()))?;
        ensure!(
            placed.trim().ends_with("/external"),
            "the external placement did not take: {placed}"
        );
        let cgroup_conflict = self.observe_owner_conflict(superseded_before)?;
        if controller.child.try_wait()?.is_none() {
            self.term_and_return(&mut controller, "recovery-cgroup-2")?;
        } else {
            self.restored(controller.child.pid())?;
            controller = self.controller("recovery-cgroup-2")?;
            self.settled()?;
        }
        let after = fs::read_to_string(format!("/proc/{}/cgroup", game.child.pid()))?;
        ensure!(
            after.trim().ends_with("/external"),
            "Stutter moved a task out of an external cgroup: {after}"
        );
        let idle = self.release(&mut controller, game, pressure)?;
        let _ = fs::remove_dir(&sibling);
        self.finish_leg(
            "external_cgroup_owner_preserved",
            json!({"after":after.trim(),"idle":idle,"autonomous_handling":cgroup_conflict}),
        );

        // The cgroup the task came from is destroyed and recreated, so it is no longer "the
        // cgroup the task left". Stutter must not write into the new one, and it must not call
        // the token settled either: the live task is still in Stutter's own placement. Only
        // when the workload is gone is there nothing left to restore.
        let (mut game, pressure) = self.applied(&mut controller, "refused-restore")?;
        let origin = self.subtree().join("suite");
        let parking = super::service_cgroup::recreate_origin(&self.subtree())?;
        let applied_before = self.probe.records.len();
        let before = controller.child.pid();
        self.signal(ServiceComponent::Controller, ServiceSignal::Term)?;
        self.await_gone(before)?;
        std::thread::sleep(Duration::from_secs(15));
        let report = self.query()?;
        ensure!(
            report["worker_tokens"]["outstanding"].as_u64() >= Some(1),
            "a live task in Stutter's placement had its token retired without being restored: {report}"
        );
        ensure!(
            fs::read_to_string(origin.join("cgroup.procs"))?
                .trim()
                .is_empty(),
            "a refused restore still wrote into the recreated cgroup"
        );
        let placed = fs::read_to_string(format!("/proc/{}/cgroup", game.child.pid()))?;
        ensure!(
            placed.trim().ends_with("/target"),
            "the task left Stutter's placement without a restore: {placed}"
        );
        self.probe.poll();
        ensure!(
            self.probe.records[applied_before..]
                .iter()
                .all(|row| row["decision"]["decision"] != "candidate_applied"),
            "Stutter applied again while an unresolved token was open"
        );
        let unresolved = report["worker_tokens"].clone();
        drop(pressure);
        stop_game(&mut game)?;
        let _ = fs::remove_dir(&parking);
        self.controller("recovery-refused-2")?;
        let idle = self.settled()?;
        self.finish_leg(
            "stale_origin_refused_then_retired_when_target_gone",
            json!({"unresolved_while_alive":unresolved,"retired":idle}),
        );

        // Suspend and resume land mid-experiment.
        self.reconfigure(Operations::Both)?;
        controller = self.controller("recovery-suspend")?;
        let (game, pressure) = self.applied(&mut controller, "suspend-game")?;
        self.binding.action(Action::SuspendResume)?;
        std::thread::sleep(Duration::from_secs(20));
        ensure!(
            controller.child.try_wait()?.is_none(),
            "suspend and resume took the always-on controller down"
        );
        let idle = self.release(&mut controller, game, pressure)?;
        self.finish_leg("suspend_resume_while_applied", json!({"idle":idle}));

        let report = self.settled()?;
        super::service::attest(&report, self.state)?;
        Ok(json!({"passed":true,"legs":self.legs,"final":report,"history":self.probe.records}))
    }

    /// The controller process with `pid` has exited (its unit may restart or stay down).
    fn await_gone(&self, pid: u32) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(40);
        while Path::new(&format!("/proc/{pid}")).exists() {
            ensure!(
                Instant::now() < deadline,
                "controller {pid} did not exit after the signal"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }
}
