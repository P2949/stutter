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
}
