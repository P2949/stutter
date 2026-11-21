//! Persisting a study's end: telemetry receipts (frozen under the observer's lock, written outside it,
//! acknowledged once durable) and a settled attempt's accounting and outcome.

use super::{
    super::{child_study::ChildStudy, report_store::write_new, study_executor::AttemptsOutcome},
    DriverStudyAttempts, STUDIES_DIR, dir_name, open_dir,
};

impl DriverStudyAttempts {
    /// Keep every attempt's telemetry receipt beside the study (write-once; a conflicting file means
    /// the receipts cannot be recorded and the study simply has none).
    pub(super) fn record_telemetry(&self, study: &ChildStudy) -> anyhow::Result<()> {
        let Some(telemetry) = &self.telemetry else {
            return Ok(());
        };
        // Freeze the batch under the lock, persist outside it, acknowledge exactly that batch.
        let receipts = telemetry
            .lock()
            .map_err(|_| anyhow::anyhow!("the telemetry observer is poisoned"))?
            .freeze_receipts();
        if receipts.is_empty() {
            return Ok(());
        }
        // A receipt that cannot be made durable is an evidence failure, not a quiet omission.
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        let bytes = serde_json::to_vec_pretty(&receipts)?;
        match write_new(&dir, "telemetry-receipts.json", &bytes) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    crate::autotune::research::report_store::read_private(
                        &dir,
                        "telemetry-receipts.json",
                        self.parent.uid
                    )? == bytes,
                    "telemetry receipts not recorded: the existing receipts differ"
                );
            }
            Err(error) => return Err(error.context("telemetry receipts not recorded")),
        }
        telemetry
            .lock()
            .map_err(|_| anyhow::anyhow!("the telemetry observer is poisoned"))?
            .acknowledge(receipts.len());
        Ok(())
    }

    /// Record a settled attempt's accounting and outcome. The driver is retained on failure, with the
    /// original terminal disposition, so a retry persists the same end and never re-drives it.
    pub(super) fn persist_end(
        &mut self,
        study: &ChildStudy,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        let Some((halted, terminal_failure)) = self.pending_end else {
            return Ok(None);
        };
        let Some((_, driver)) = self.running.as_ref() else {
            self.pending_end = None;
            return Ok(None);
        };
        // What this child used is recorded before anything else is decided about it, so the next
        // child starts from the campaign's true spend; unrecorded fails closed.
        let result = self
            .record_budget(study, driver)
            .and_then(|()| self.outcome(study, driver, halted, terminal_failure));
        match result {
            Ok(outcome) => {
                self.running = None;
                self.pending_end = None;
                self.persist_failures = 0;
                Ok(Some(outcome))
            }
            Err(error) => {
                self.persist_failures += 1;
                Err(error)
            }
        }
    }

    /// What the gate saw during an Action study: an applied action, decided, with its restoration
    /// proven. Anything less (nothing applied, or an action still outstanding) is not evidence.
    pub(super) fn stage_evidence(&self, study: &ChildStudy) -> Option<bool> {
        use crate::autotune::research::child_study::Stage;
        let (Some(gate), Stage::Action(_)) = (&self.gate, study.stage) else {
            return None;
        };
        let evidence = gate.lock().ok()?.disarm();
        // The receipts must be durable beside the study, where an independent verification reads them:
        // a failed or conflicting write is a failed qualification, never a silent one.
        let durable = !evidence.receipts.is_empty()
            && self
                .write_receipts(study, &evidence.receipts_json())
                .is_ok();
        Some(
            evidence.clean
                && !evidence.settled.is_empty()
                && evidence.kernel_state_proven
                && durable,
        )
    }

    /// A feature study's per-attempt observation, recorded once beside the study (an existing record wins).
    pub(super) fn feature_receipt(&self, study_dir: &std::fs::File) -> anyhow::Result<()> {
        if self.feature_observed.is_empty() {
            return Ok(());
        }
        let bytes =
            crate::autotune::research::init_receipt::feature_receipt_bytes(&self.feature_observed)?;
        match write_new(
            study_dir,
            crate::autotune::research::init_receipt::FEATURE_RECEIPT,
            &bytes,
        ) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The initialization observation, recorded once beside the study so an independent verification reads
    /// exactly what the live judgement stood on. An existing record wins (a retry never re-decides it).
    pub(super) fn init_receipt(&self, study_dir: &std::fs::File) -> anyhow::Result<Option<bool>> {
        let arms: Vec<&str> = self
            .parent
            .prefix_init_arms
            .iter()
            .map(|arm| arm.dir_name())
            .collect();
        if let Some(observed) = self.init_observed {
            let bytes =
                serde_json::to_vec(&serde_json::json!({ "arms": arms, "all_warm": observed }))?;
            match write_new(
                study_dir,
                crate::autotune::research::init_receipt::INIT_RECEIPT,
                &bytes,
            ) {
                Ok(()) => return Ok(Some(observed)),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        match crate::autotune::research::report_store::read_private(
            study_dir,
            crate::autotune::research::init_receipt::INIT_RECEIPT,
            self.parent.uid,
        ) {
            Ok(bytes) => Ok(crate::autotune::research::init_receipt::read_init_receipt(
                &bytes, &arms,
            )),
            Err(_) => Ok(None),
        }
    }
}
