//! Packaged-service lifecycle checks within the shared round driver.
use super::*;

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

    /// Tokens the root worker has recorded as unresolved so far.
    pub(super) fn unresolved_tokens(&self) -> u64 {
        self.world
            .service
            .as_ref()
            .and_then(|binding| {
                binding
                    .action(stutter_validation_runner::model::BoundedServiceAction::Query)
                    .ok()
            })
            .and_then(|report| {
                report["worker_tokens"]["recovery_audit"]["dispositions"]["unresolved"].as_u64()
            })
            .unwrap_or(0)
    }

    /// The root ledger's unsettled tokens, each told apart by its plan's creation time.
    pub(super) fn outstanding_token_ids(&self) -> Vec<String> {
        self.world
            .service
            .as_ref()
            .and_then(|binding| {
                binding
                    .action(stutter_validation_runner::model::BoundedServiceAction::Query)
                    .ok()
            })
            .and_then(|report| {
                report["worker_tokens"]["outstanding_ids"]
                    .as_array()
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| id.as_str().map(str::to_owned))
                            .collect()
                    })
            })
            .unwrap_or_default()
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
                let still_open: Vec<_> = interrupted
                    .iter()
                    .filter(|id| {
                        report["worker_tokens"]["outstanding_ids"]
                            .as_array()
                            .is_some_and(|open| open.iter().any(|open| open == id.as_str()))
                    })
                    .collect();
                anyhow::ensure!(
                    still_open.is_empty(),
                    "the interrupted root token {still_open:?} is still outstanding: {}",
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
