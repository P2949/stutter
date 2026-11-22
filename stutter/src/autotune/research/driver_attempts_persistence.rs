//! Persisting a study's end: telemetry receipts (frozen under the observer's lock, written outside it,
//! acknowledged once durable) and a settled attempt's accounting and outcome.

use super::{
    super::{
        action_child::ActionChildScope,
        action_coverage::ABSTENTION_RECORD,
        child_study::{ChildStudy, Stage},
        report_store::write_new,
        study_executor::AttemptsOutcome,
    },
    DriverStudyAttempts, STUDIES_DIR, dir_name, open_dir,
};

impl DriverStudyAttempts {
    /// An Action study arms the gate (once) on the game process the host has sighted and attested,
    /// authorizing only that study's action, for the rest of the campaign's window.
    pub(super) fn arm_gate_for(&mut self, study: &ChildStudy) {
        let (Some(gate), Stage::Action(action)) = (&self.gate, study.stage) else {
            return;
        };
        if self.armed_for.as_deref() != Some(study.study_sha256.as_str())
            && let Some(target) = self.host.game_process()
        {
            let scope = ActionChildScope::new([action], target, self.parent.deadline_unix_seconds);
            if gate.lock().is_ok_and(|mut gate| gate.arm(scope)) {
                self.armed_for = Some(study.study_sha256.clone());
                self.armed_target = Some(target);
            }
        } else if let Some(target) = self.host.game_process()
            && self.armed_target != Some(target)
            && gate.lock().is_ok_and(|mut gate| gate.rebind(target))
        {
            // A later attempt is a new game process: the action is scoped to that process alone,
            // never inherited from the previous launch.
            self.armed_target = Some(target);
        }
        // In a treatment study the action may be applied only while an attempt treated On runs; this
        // is set after arming, which starts from permitting.
        if self.treatment_study {
            let on = self.host.attempt_treatment() == Some(super::super::Treatment::On);
            if let Ok(mut gate) = gate.lock() {
                gate.set_attempt_on(on);
            }
        }
    }

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
                self.applied_settings.clear();
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
    /// As above, and whether the action study justified abstaining: the gate armed, nothing was applied or
    /// left outstanding, every granted action (none) restored, and that was made durable beside the study.
    pub(super) fn stage_evidence(
        &self,
        study: &ChildStudy,
        completed: u32,
    ) -> (Option<bool>, bool) {
        use crate::autotune::research::child_study::Stage;
        let (Some(gate), Stage::Action(_)) = (&self.gate, study.stage) else {
            return (None, false);
        };
        let Some(evidence) = gate.lock().ok().map(|mut gate| gate.disarm()) else {
            return (None, false);
        };
        // The receipts must be durable beside the study, where an independent verification reads them:
        // a failed or conflicting write is a failed qualification, never a silent one.
        let durable = !evidence.receipts.is_empty()
            && self
                .write_receipts(study, &evidence.receipts_json())
                .is_ok();
        let qualified = evidence.clean
            && !evidence.settled.is_empty()
            && evidence.kernel_state_proven
            && durable;
        // Nothing applied, nothing outstanding and nothing to restore: the controller abstained for the whole
        // study. That is recorded (and replayed) as its own disposition, never as a qualified action.
        let abstained = !qualified
            && evidence.receipts.is_empty()
            && evidence.settled.is_empty()
            && evidence.clean
            && self
                .write_study_record(
                    study,
                    ABSTENTION_RECORD,
                    &serde_json::json!({ "completed": completed, "applied": 0 }),
                )
                .is_ok();
        (Some(qualified), abstained)
    }

    /// A small JSON record beside the study, write-once; an existing one must be byte-identical.
    fn write_study_record(
        &self,
        study: &ChildStudy,
        name: &str,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        let bytes = serde_json::to_vec_pretty(value)?;
        match write_new(&dir, name, &bytes) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    super::super::report_store::read_private(&dir, name, self.parent.uid)? == bytes,
                    "the recorded study record differs"
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The settings digests the applied profile left, recorded once beside the study (an existing record
    /// wins), and returned; empty when no profile was applied.
    pub(super) fn settings_receipt(
        &self,
        study_dir: &std::fs::File,
    ) -> anyhow::Result<Vec<String>> {
        use crate::autotune::research::init_receipt::{
            SETTINGS_RECEIPT, read_settings_receipt, settings_receipt_bytes,
        };
        if !self.applied_settings.is_empty() {
            match write_new(
                study_dir,
                SETTINGS_RECEIPT,
                &settings_receipt_bytes(&self.applied_settings)?,
            ) {
                Ok(()) => return Ok(self.applied_settings.clone()),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(crate::autotune::research::report_store::read_private(
            study_dir,
            SETTINGS_RECEIPT,
            self.parent.uid,
        )
        .ok()
        .and_then(|bytes| read_settings_receipt(&bytes))
        .unwrap_or_default())
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
