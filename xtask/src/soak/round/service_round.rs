//! Packaged-service lifecycle checks within the shared round driver.
use super::*;

/// The root ledger as it stood just before a fault was injected.
#[derive(Default)]
pub(super) struct CrashBaseline {
    pub(super) unresolved: u64,
    pub(super) interrupted: Vec<String>,
}

/// The controller resolved its own experiment (a focus loss, say) before the fault could be
/// injected. No fault was injected, so the round proves nothing either way and is repeated.
#[derive(Debug)]
pub(super) struct ExperimentAlreadyResolved;

impl std::fmt::Display for ExperimentAlreadyResolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the controller resolved its experiment before the fault was injected")
    }
}

impl std::error::Error for ExperimentAlreadyResolved {}

impl Ctx<'_> {
    pub(super) fn worker_kill_round(
        &mut self,
        mut controller: Controller,
        mut game: Game,
    ) -> anyhow::Result<()> {
        let tried = self.applied(&mut controller)?;
        let worker_recoveries = if let Some(service) = &self.world.service {
            service.action(stutter_validation_runner::model::BoundedServiceAction::Query)?["worker_tokens"]["recovery_audit"]["completed"].as_u64().context("worker recovery count missing")?
        } else {
            0
        };
        self.world.kill_worker()?;
        self.event("worker_kill", "kill -9 of the privileged worker");
        std::thread::sleep(Duration::from_secs(3));
        if self.world.ensure_worker()? {
            self.event("worker_start", "privileged worker restarted");
        }
        if self.world.service.is_some() {
            self.end_session(controller, "faulted_with_worker");
            controller = self.start_controller(&game, "controller-2", self.round.operations)?;
            self.check_restored(&game);
            let report = self
                .world
                .service
                .as_ref()
                .context("service binding missing")?
                .action(stutter_validation_runner::model::BoundedServiceAction::Query)?;
            anyhow::ensure!(
                report["worker_tokens"]["outstanding"] == 0
                    && report["worker_tokens"]["recovery_audit"]["completed"]
                        .as_u64()
                        .is_some_and(|count| count > worker_recoveries),
                "worker restart did not durably audit and settle its issued token"
            );
            fs::write(
                self.dir.join("worker-recovery-runtime.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            self.event(
                "worker_recovery",
                "new root recovery audit and physical task restoration verified",
            );
            self.memory_holds(&mut controller, tried)?;
            return self.end_workload(&mut game, controller, "workload_exit");
        }
        if self.resolution(&mut controller)?.is_none() && controller.child.try_wait()?.is_some() {
            // The controller faulted with the worker; the supervisor starts another.
            self.end_session(controller, "faulted_with_worker");
            controller = self.start_controller(&game, "controller-2", self.round.operations)?;
            self.experiment(&mut controller, &game)?;
        }
        self.end_workload(&mut game, controller, "workload_exit")
    }

    /// What the root ledger holds the instant before a fault is injected into an applied
    /// experiment: how many tokens root has left unresolved so far, and which tokens are
    /// outstanding (told apart by their plan's creation time). One query, so no time passes
    /// between the proof that the experiment is still applied and the fault. A controller that
    /// already resolved its own experiment (for example a focus loss rolled it back) leaves
    /// nothing for the fault to interrupt, and the round says so instead of failing.
    pub(super) fn crash_baseline(&self) -> anyhow::Result<CrashBaseline> {
        let Some(binding) = &self.world.service else {
            return Ok(CrashBaseline::default());
        };
        let report =
            binding.action(stutter_validation_runner::model::BoundedServiceAction::Query)?;
        let tokens = &report["worker_tokens"];
        anyhow::ensure!(
            tokens["readable"] == true && tokens["recovery_audit"]["readable"] == true,
            "the root token ledger or its audit was unreadable before the fault: {tokens}"
        );
        let outstanding = tokens["outstanding"].as_u64().unwrap_or(0);
        if outstanding == 0 {
            return Err(anyhow::Error::new(ExperimentAlreadyResolved));
        }
        let interrupted: Vec<String> = tokens["outstanding_ids"]
            .as_array()
            .context("the root report names no outstanding token ids")?
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        anyhow::ensure!(
            interrupted.len() as u64 == outstanding,
            "the root report named {} of {outstanding} outstanding tokens",
            interrupted.len()
        );
        Ok(CrashBaseline {
            unresolved: tokens["recovery_audit"]["dispositions"]["unresolved"]
                .as_u64()
                .unwrap_or(0),
            interrupted,
        })
    }

    pub(super) fn wait_service_recovery(
        &mut self,
        game: &Game,
        startup_before: usize,
        emergency_before: usize,
        unresolved_before: u64,
        interrupted: &[String],
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            let startup = audit_events(&self.world.audit_path(), "autotune-startup-recovery");
            let emergency = audit_events(&self.world.audit_path(), "autotune emergency restore");
            let events: Vec<_> = startup
                .iter()
                .skip(startup_before)
                .chain(emergency.iter().skip(emergency_before))
                .collect();
            if !events.is_empty() {
                anyhow::ensure!(
                    events.iter().all(|(success, _)| *success),
                    "service crash recovery reported a failed restore"
                );
                let report = self
                    .world
                    .service
                    .as_ref()
                    .context("service binding missing")?
                    .action(stutter_validation_runner::model::BoundedServiceAction::Query)?;
                // Once the restarted controller holds a token for a new experiment, the workload
                // legitimately carries that experiment, so the physical readback only applies while
                // the ledger is empty. The audit and root disposition checks below always apply.
                if report["worker_tokens"]["outstanding"] == 0 {
                    self.check_restored(game);
                }
                // The restarted controller may already hold a token for a new experiment; what
                // matters is that root settled the interrupted one and left nothing new
                // unresolved (the counter is cumulative over the campaign).
                anyhow::ensure!(
                    report["worker_tokens"]["readable"] == true
                        && report["worker_tokens"]["recovery_audit"]["dispositions"]["unresolved"]
                            .as_u64()
                            == Some(unresolved_before),
                    "root worker has an unsettled crash token: {}",
                    report["worker_tokens"]
                );
                // The token the interruption left behind is settled by name; a token the restarted
                // controller issued since is a different one and may still be open.
                let ids = |key: &str| -> anyhow::Result<Vec<String>> {
                    Ok(report["worker_tokens"][key]
                        .as_array()
                        .with_context(|| format!("the root report names no {key}"))?
                        .iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect())
                };
                let (open, settled) = (ids("outstanding_ids")?, ids("settled_ids")?);
                let unsettled: Vec<_> = interrupted
                    .iter()
                    .filter(|id| open.contains(id) || !settled.contains(id))
                    .collect();
                anyhow::ensure!(
                    unsettled.is_empty(),
                    "the interrupted root token {unsettled:?} was not observed settled: {}",
                    report["worker_tokens"]
                );
                fs::write(
                    self.dir.join("crash-recovery-runtime.json"),
                    serde_json::to_vec_pretty(&report)?,
                )?;
                self.event("service_recovery", "new successful startup or stop-hook audit, physical restoration and root ledger readback");
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "service crash left no new restoration audit"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    pub(super) fn service_idle(&mut self, controller: &mut Controller) -> anyhow::Result<()> {
        let report = crate::soak::service_idle::wait(self.world, controller)?;
        fs::write(
            self.dir.join(format!("idle-runtime-{}.json", unix_ms())),
            serde_json::to_vec_pretty(&report)?,
        )?;
        self.event(
            "service_idle_after_exit",
            "same controller stayed alive; controller and root token ledgers settled",
        );
        Ok(())
    }

    pub(super) fn finish_service_workload(
        &mut self,
        mut controller: Controller,
        ended_by: &str,
    ) -> anyhow::Result<()> {
        self.service_idle(&mut controller)?;
        self.record.sessions.push(SessionRecord {
            start_unix_ms: controller.started_unix_ms,
            end_unix_ms: unix_ms(),
            exit_code: None,
            signal: None,
            ended_by: format!("service_idle_{ended_by}"),
        });
        Ok(())
    }
}
