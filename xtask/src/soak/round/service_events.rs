//! The service-only perturbations of the soak: administrator, machine and neighbour events that
//! the historical direct-controller schedule cannot express. Each one ends with the same proof as
//! every other round: nothing applied, nothing outstanding, and nobody else's state overwritten.
use std::process::Command;

use stutter_validation_runner::model::BoundedServiceAction as Action;

use super::*;

impl Ctx<'_> {
    fn binding(&self) -> anyhow::Result<&super::super::service::Binding> {
        self.world
            .service
            .as_ref()
            .context("service binding missing")
    }

    fn subtree(&self) -> anyhow::Result<PathBuf> {
        Ok(Path::new("/sys/fs/cgroup").join(format!(
            "stutter-validation-{}",
            self.binding()?.campaign.campaign_id
        )))
    }

    /// Every root-issued token is settled and the task ledger is gone.
    fn tokens_settled(&mut self) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let report = self.binding()?.action(Action::Query)?;
            if report["worker_tokens"]["readable"] == true
                && report["worker_tokens"]["outstanding"] == 0
                && !self.world.ledger_path().exists()
            {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "root tokens or the task ledger were not settled"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn term_controller(&mut self, controller: &mut Controller) {
        if let Err(error) = controller.child.signal(SIGTERM) {
            self.violation(format!("controller SIGTERM failed: {error:#}"));
        }
    }

    /// Stops the applied controller in the given way and brings it back on the same workload.
    fn restarted_after(
        &mut self,
        mut controller: Controller,
        game: &Game,
        how: &str,
    ) -> anyhow::Result<Controller> {
        let startup_before =
            audit_events(&self.world.audit_path(), "autotune-startup-recovery").len();
        let emergency_before =
            audit_events(&self.world.audit_path(), "autotune emergency restore").len();
        match how {
            "sigterm" => self.term_controller(&mut controller),
            "restart" => {
                self.binding()?.action(Action::Restart)?;
            }
            _ => self.kill_controller(&mut controller),
        }
        self.end_session(controller, how);
        let next = self.start_controller(game, "controller-2", self.round.operations)?;
        self.wait_service_recovery(game, startup_before, emergency_before)?;
        Ok(next)
    }

    pub(super) fn controller_sigterm_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        let tried = self.applied(&mut controller)?;
        self.event("controller_sigterm", "SIGTERM while a candidate is applied");
        let mut next = self.restarted_after(controller, &game, "sigterm")?;
        self.memory_holds(&mut next, tried)?;
        self.end_workload(&mut game, next, "workload_exit")
    }

    pub(super) fn service_restart_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        let tried = self.applied(&mut controller)?;
        self.event(
            "service_restart",
            "systemctl restart while a candidate is applied",
        );
        let mut next = self.restarted_after(controller, &game, "restart")?;
        self.memory_holds(&mut next, tried)?;
        self.end_workload(&mut game, next, "workload_exit")
    }

    /// The only workload is an unqualified game: the always-on controller must not mutate it.
    pub(super) fn focus_unqualified_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        signal(game.child.pid(), SIGTERM);
        let _ = wait_exit(&mut game.child, Duration::from_secs(10));
        self.service_idle(&mut controller)?;
        let weak = self
            .dir
            .join("low-confidence/steamapps/common/WeakGame/WeakGame");
        fs::create_dir_all(weak.parent().context("fixture parent missing")?)?;
        fs::copy(&self.world.xtask, &weak)?;
        let qualified = std::mem::replace(&mut self.world.game, weak);
        let spawned = self.world.spawn_game(&self.dir, "weak-game", 60);
        self.world.game = qualified;
        let mut weak_game = spawned?;
        self.event("focus_unqualified", "an unqualified game holds the focus");
        let seen_before = self.seen.len();
        let deadline = Instant::now() + Duration::from_secs(self.timing.candidate_window_s + 25);
        while Instant::now() < deadline {
            self.poll_events();
            std::thread::sleep(Duration::from_millis(500));
        }
        if self.seen[seen_before..]
            .iter()
            .any(|event| event.decision == "candidate_applied")
        {
            self.violation("an unqualified focus was mutated");
        }
        signal(weak_game.child.pid(), SIGTERM);
        let _ = wait_exit(&mut weak_game.child, Duration::from_secs(10));
        self.finish_service_workload(controller, "workload_exit")
    }

    /// Someone else changes the task after Stutter did; their value must survive Stutter's stop.
    pub(super) fn external_mutation_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        self.applied(&mut controller)?;
        let pid = game.child.pid();
        let cgroup_mode = self.round.operations == Operations::Cgroup;
        let observe = move |pid: u32| -> anyhow::Result<String> {
            if cgroup_mode {
                Ok(fs::read_to_string(format!("/proc/{pid}/cgroup"))?
                    .trim()
                    .to_owned())
            } else {
                Ok(task_allowed_lists(pid).join("|"))
            }
        };
        let external = if self.round.operations == Operations::Cgroup {
            let sibling = self.subtree()?.join("external");
            fs::create_dir_all(&sibling)?;
            fs::write(sibling.join("cgroup.procs"), pid.to_string())?;
            observe(pid)?
        } else {
            let both = format!("{},{}", self.world.cpus.0, self.world.cpus.1);
            let status = Command::new("taskset")
                .args(["-a", "-p", "-c", &both])
                .arg(pid.to_string())
                .output()?;
            anyhow::ensure!(status.status.success(), "the external mutation failed");
            observe(pid)?
        };
        self.event(
            "external_mutation",
            format!("external owner now at {external}"),
        );
        let seen_before = self.seen.len();
        std::thread::sleep(Duration::from_secs(45));
        self.poll_events();
        if self.seen[seen_before..]
            .iter()
            .any(|event| event.decision == "candidate_applied")
        {
            self.violation("the controller applied again while an external owner held the task");
        }
        self.term_controller(&mut controller);
        self.end_session(controller, "sigterm_after_external_mutation");
        self.tokens_settled()?;
        let after = observe(pid)?;
        if after != external {
            self.violation(format!(
                "Stutter overwrote an external owner: was {external}, now {after}"
            ));
        }
        let mut next = self.start_controller(&game, "controller-2", self.round.operations)?;
        self.service_idle_after_external(&mut next)?;
        self.end_workload(&mut game, next, "workload_exit")
    }

    /// With an external owner the journal may legitimately stay unsettled for the controller; the
    /// root ledger must be settled and the controller alive.
    fn service_idle_after_external(&mut self, controller: &mut Controller) -> anyhow::Result<()> {
        anyhow::ensure!(
            controller.child.try_wait()?.is_none(),
            "the restarted controller exited after an external mutation"
        );
        self.tokens_settled()
    }

    pub(super) fn corrupt_journal_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        self.applied(&mut controller)?;
        self.kill_controller(&mut controller);
        self.end_session(controller, "kill9");
        let journal = self.world.journal_path();
        fs::write(&journal, b"{ corrupted by the soak")?;
        self.event("corrupt_journal", "the controller journal was corrupted");
        // Safety does not wait for a human: the worker settles its own ledger on the way in.
        self.tokens_settled()?;
        fs::rename(&journal, journal.with_extension("json.corrupt"))?;
        self.check_restored(&game);
        let next = self.start_controller(&game, "controller-2", self.round.operations)?;
        self.end_workload(&mut game, next, "workload_exit")
    }

    /// The cgroup the task came from is destroyed and recreated, so restoring into it is refused.
    /// The live task is still in Stutter's own placement, so the token must stay unresolved until
    /// the workload is gone; only then is there nothing left to restore.
    pub(super) fn failed_restore_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        self.applied(&mut controller)?;
        let origin = self.subtree()?.join("suite");
        let parking = super::super::service_cgroup::recreate_origin(&self.subtree()?)?;
        self.event("failed_restore", "the original cgroup was recreated");
        let seen_before = self.seen.len();
        self.term_controller(&mut controller);
        self.end_session(controller, "sigterm_with_unrestorable_origin");
        std::thread::sleep(Duration::from_secs(15));
        self.poll_events();
        let report = self.binding()?.action(Action::Query)?;
        if report["worker_tokens"]["outstanding"].as_u64() < Some(1) {
            self.violation("a live task in Stutter's placement had its token retired unrestored");
        }
        if !fs::read_to_string(origin.join("cgroup.procs"))?
            .trim()
            .is_empty()
        {
            self.violation("a refused restore still wrote into the recreated cgroup");
        }
        if self.seen[seen_before..]
            .iter()
            .any(|event| event.decision == "candidate_applied")
        {
            self.violation("Stutter applied again while an unresolved token was open");
        }
        signal(game.child.pid(), SIGTERM);
        let _ = wait_exit(&mut game.child, Duration::from_secs(10));
        self.event(
            "game_exit",
            "the workload left, so nothing remains to restore",
        );
        let _ = fs::remove_dir(&parking);
        let next = self.start_controller(&game, "controller-2", self.round.operations)?;
        self.tokens_settled()?;
        self.finish_service_workload(next, "workload_exit")
    }
}
