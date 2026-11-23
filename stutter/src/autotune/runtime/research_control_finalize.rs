//! Two-phase completion: reconstruct while retaining the lease, then verify the released artifacts.
use super::*;

struct PreparedFinalization {
    dir: File,
    bytes: Vec<u8>,
    phase: ResearchPhase,
    code: &'static str,
    cause: Option<crate::autotune::research::driver::FailureClass>,
}

fn write_identical(dir: &File, uid: u32, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    use crate::autotune::research::report_store::{read_private, write_new};
    match write_new(dir, name, bytes) {
        Ok(()) => {
            anyhow::ensure!(
                read_private(dir, name, uid)? == bytes,
                "finalization record did not persist"
            );
            Ok(())
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            anyhow::ensure!(
                read_private(dir, name, uid)? == bytes,
                "finalization record conflicts"
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Validate the durable preterminal binding without asserting its scientific verdict.
pub(in crate::autotune::runtime) fn prepared_record(
    dir: &File,
    uid: u32,
    intent_sha256: &str,
) -> anyhow::Result<serde_json::Value> {
    use crate::autotune::research::report_store::read_private;
    let report = read_private(dir, "report.json", uid)?;
    let record: serde_json::Value =
        serde_json::from_slice(&read_private(dir, "finalization-prepared.json", uid)?)?;
    anyhow::ensure!(
        record["schema"] == 1
            && record["intent_sha256"] == intent_sha256
            && record["report_sha256"].as_str()
                == Some(stutter_core::hash::sha256_hex(&report).as_str())
            && matches!(
                record["target_phase"].as_str(),
                Some("completed" | "blocked")
            )
            && record["verdict"].is_object(),
        "prepared finalization binding differs"
    );
    Ok(record)
}

/// Check the durable decision and historical bundle before treating a recovered completion as qualified.
pub(super) fn verify_completed_receipt(
    dir: &File,
    uid: u32,
    report: &crate::autotune::research::report::ResearchReportV1,
) -> anyhow::Result<()> {
    use crate::autotune::research::report_store::read_private;
    let report_bytes = read_private(dir, "report.json", uid)?;
    let prepared_bytes = read_private(dir, "finalization-prepared.json", uid)?;
    let prepared: serde_json::Value = serde_json::from_slice(&prepared_bytes)?;
    let receipt: serde_json::Value =
        serde_json::from_slice(&read_private(dir, "finalization-verified.json", uid)?)?;
    anyhow::ensure!(
        prepared["schema"] == 1
            && receipt["schema"] == 1
            && prepared["intent_sha256"] == report.intent_sha256
            && receipt["intent_sha256"] == report.intent_sha256
            && prepared["target_phase"] == "completed"
            && prepared["terminal_failure"].is_null()
            && receipt["terminal_phase"] == "completed"
            && prepared["verdict"]["final_gate_qualified"] == true
            && receipt["verdict"]["final_gate_qualified"] == true
            && prepared["report_sha256"].as_str()
                == Some(stutter_core::hash::sha256_hex(&report_bytes).as_str())
            && receipt["prepared_sha256"].as_str()
                == Some(stutter_core::hash::sha256_hex(&prepared_bytes).as_str()),
        "completed finalization receipt differs"
    );
    let before = prepared["verdict"]["gates"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("prepared gates missing"))?;
    let after = receipt["verdict"]["gates"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("verified gates missing"))?;
    let required = [
        "reconstruction_passed",
        "raw_files_verified",
        "loaded_runtime_qualified",
        "child_studies_qualified",
        "child_reports_verified",
        "restoration_verified",
    ];
    for verdict in [&prepared["verdict"], &receipt["verdict"]] {
        anyhow::ensure!(
            required.iter().all(|field| verdict[*field] == true)
                && verdict["disposition"] == "Qualified",
            "completed verdict is internally inconsistent"
        );
    }
    let expected = ["G0", "G1", "G2", "G3", "G4", "G5", "G6", "G7", "G8", "G9"];
    anyhow::ensure!(
        before.len() == expected.len()
            && after.len() == expected.len()
            && before
                .iter()
                .zip(&expected)
                .all(|(gate, name)| gate["gate"] == *name
                    && gate["accepted"] == true
                    && gate["blockers"].as_array().is_some_and(Vec::is_empty))
            && after
                .iter()
                .zip(&expected)
                .all(|(gate, name)| gate["gate"] == *name
                    && gate["accepted"] == true
                    && gate["blockers"].as_array().is_some_and(Vec::is_empty))
            && before
                .iter()
                .filter(|gate| gate["gate"] != "G1")
                .eq(after.iter().filter(|gate| gate["gate"] != "G1")),
        "completed gate receipt differs"
    );
    let bundle = receipt["bundle"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("completed bundle missing"))?;
    crate::autotune::research::final_bundle::verify_manifest(
        dir,
        uid,
        bundle,
        &serde_json::to_value(report)?,
        &receipt["verdict"],
    )
}

impl ResearchCoordinator {
    pub(super) fn terminal_preparation_exists(&self) -> bool {
        let Some(dir) = self.open_campaign_study_dir() else {
            return false;
        };
        match crate::autotune::research::report_store::read_private(
            &dir,
            "finalization-prepared.json",
            crate::syscall::geteuid(),
        ) {
            Ok(_) => true,
            Err(error) => !error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound),
        }
    }
    pub(in crate::autotune::runtime) fn finalization_due(&self) -> bool {
        self.storage_blocker != Some("UnsafeToResume")
            && (self.terminal_recheck_pending
                || (self.driver.is_none()
                    && self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.lease.phase() == ResearchPhase::Restoring)))
    }

    pub(in crate::autotune::runtime) fn drive_finalization(&mut self, now: u64) {
        if self.terminal_recheck_pending {
            self.recheck_recovered_terminal();
        } else {
            self.finalize_done(now);
        }
    }

    fn recheck_recovered_terminal(&mut self) {
        let uid = crate::syscall::geteuid();
        let result = (|| -> anyhow::Result<&'static str> {
            let terminal = self
                .terminal
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("terminal state missing"))?;
            anyhow::ensure!(
                matches!(
                    terminal.phase,
                    ResearchPhase::Completed | ResearchPhase::Blocked
                ),
                "unsupported recovered finalization"
            );
            let dir = self
                .open_campaign_study_dir()
                .ok_or_else(|| anyhow::anyhow!("campaign study unavailable"))?;
            let prepared = crate::autotune::research::report_store::read_private(
                &dir,
                "finalization-prepared.json",
                uid,
            )?;
            let decision = prepared_record(&dir, uid, &terminal.intent_sha256)?;
            let code = if decision["verdict"]["disposition"] == "AbortedHealth" {
                "BlockedGpuStability"
            } else if terminal.phase == ResearchPhase::Completed {
                "QualifiedEndToEnd"
            } else if decision["verdict"]["disposition"] == "PartiallyQualified" {
                "PartiallyQualified"
            } else {
                "BlockedCampaignOrStats"
            };
            if terminal.phase == ResearchPhase::Blocked {
                self.verify_terminal_artifacts(&dir, uid, &prepared, ResearchPhase::Blocked, code)?;
                return Ok(code);
            }
            match crate::autotune::research::report_store::read_private(
                &dir,
                "finalization-verified.json",
                uid,
            ) {
                Ok(_) => {
                    let report: crate::autotune::research::report::ResearchReportV1 =
                        serde_json::from_slice(
                            &crate::autotune::research::report_store::read_private(
                                &dir,
                                "report.json",
                                uid,
                            )?,
                        )?;
                    verify_completed_receipt(&dir, uid, &report)?;
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
                {
                    self.verify_terminal_artifacts(
                        &dir,
                        uid,
                        &prepared,
                        ResearchPhase::Completed,
                        "QualifiedEndToEnd",
                    )?;
                }
                Err(error) => return Err(error),
            }
            let current: serde_json::Value =
                serde_json::from_str(&self.report_response(true, uid))?;
            anyhow::ensure!(
                current["historical_finalization_verified"] == true
                    && current["reconstruction"] == "passed",
                "historical completion did not verify"
            );
            Ok(
                if current["verdict"]["final_gate_qualified"] == true && current["status"] == "ok" {
                    "QualifiedEndToEnd"
                } else {
                    "PartiallyQualified"
                },
            )
        })();
        self.terminal_recheck_pending = false;
        match result {
            Ok(code) => {
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.code = code;
                    terminal.report_persisted = Some(true);
                    terminal.finalization_verified = Some(true);
                }
            }
            Err(error) => {
                log::error!("recovered terminal verification failed: {error:#}");
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.code = "TerminalVerificationFailed";
                    terminal.finalization_verified = Some(false);
                }
                self.storage_blocker = Some("UnsafeToResume");
            }
        }
    }

    /// Called on the existing research-management worker, with the action slot reserved.
    pub(in crate::autotune::runtime) fn finalize_done(&mut self, now: u64) {
        let resumed = self.driver.is_none()
            && self
                .active
                .as_ref()
                .is_some_and(|active| active.lease.phase() == ResearchPhase::Restoring);
        if !resumed && !self.driver_is_quiescent() {
            self.storage_blocker = Some("UnsafeToResume");
            return;
        }
        if !resumed {
            match self.settle_owned_driver(now) {
                Ok(None) => {}
                Ok(Some(class)) => {
                    let phase =
                        if class == crate::autotune::research::driver::FailureClass::Integrity {
                            ResearchPhase::Aborted
                        } else {
                            ResearchPhase::Blocked
                        };
                    let persisted = self.persist_report();
                    self.settle(now, phase, class.terminal_code(), Some(class));
                    if let Some(terminal) = self.terminal.as_mut() {
                        terminal.report_persisted = Some(persisted);
                    }
                    return;
                }
                Err(error) => {
                    log::error!("research finalization could not settle ownership: {error:#}");
                    self.storage_blocker = Some("UnsafeToResume");
                    return;
                }
            }
            if !self.persist_report() {
                self.settle(now, ResearchPhase::Blocked, "ReportPersistenceFailed", None);
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.report_persisted = Some(false);
                }
                return;
            }
            let Some(active) = self.active.as_mut() else {
                return;
            };
            if active.lease.advance(ResearchPhase::Restoring, now).is_err() {
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
            // The attempt and child journals must be unlocked for independent reconstruction, while
            // the separate campaign lease remains held by this coordinator.
            drop(self.driver.take());
        }
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let uid = active.lease.owner_uid();
        if resumed {
            let valid = self
                .open_campaign_study_dir()
                .and_then(|dir| prepared_record(&dir, uid, &active.intent_sha256).ok())
                .is_some();
            if !valid {
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
        }
        let preparation = (|| -> anyhow::Result<PreparedFinalization> {
            let dir = self
                .open_campaign_study_dir()
                .ok_or_else(|| anyhow::anyhow!("campaign study unavailable"))?;
            // The protected-state seal is part of this durable preterminal step: written once, here, while the lease is
            // still held, before the evidence that reads it and bound by digest into the decision below.
            let intent_for_seal = self
                .active
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("lease lost"))?
                .intent_sha256
                .clone();
            // The seal outcome is kept: a campaign cannot complete on a seal that was not written or is not valid.
            let sealed = self.host_sources.0.as_deref().map(|steamapps| {
                crate::autotune::research::protected_state::seal_unchanged(
                    steamapps,
                    &dir,
                    uid,
                    &intent_for_seal,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_secs()),
                )
            });
            // The crosswalk's observations, derived from the children's own sealed raw windows before the report reads them.
            if let Err(error) =
                crate::autotune::research::crosswalk_producer::produce(&dir, uid, &intent_for_seal)
            {
                log::warn!("crosswalk observations were not produced: {error:#}");
            }
            let seal_digest = crate::autotune::research::protected_state::seal_digest(
                &dir,
                uid,
                &intent_for_seal,
            );
            let seal_valid = matches!(sealed, Some(Ok(())))
                && seal_digest.is_some()
                && crate::autotune::research::protected_state::sealed(&dir, uid, &intent_for_seal)
                    .is_ok();
            let report =
                crate::autotune::research::report_store::read_private(&dir, "report.json", uid)?;
            let evidence: serde_json::Value =
                serde_json::from_str(&self.finalization_evidence(uid))?;
            anyhow::ensure!(
                evidence["reconstruction"] == "passed",
                "preterminal reconstruction failed"
            );
            let verdict = evidence
                .get("verdict")
                .filter(|value| value.is_object())
                .ok_or_else(|| anyhow::anyhow!("preterminal verdict missing"))?;
            let qualified = verdict["final_gate_qualified"]
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("preterminal qualification missing"))?;
            let (phase, code, cause) = if verdict["disposition"] == "AbortedHealth" {
                (
                    ResearchPhase::Blocked,
                    "BlockedGpuStability",
                    Some(crate::autotune::research::driver::FailureClass::Health),
                )
            } else if qualified && seal_valid {
                (ResearchPhase::Completed, "QualifiedEndToEnd", None)
            } else if verdict["disposition"] == "PartiallyQualified" {
                (ResearchPhase::Blocked, "PartiallyQualified", None)
            } else {
                (ResearchPhase::Blocked, "BlockedCampaignOrStats", None)
            };
            let intent = self
                .active
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("lease lost"))?
                .intent_sha256
                .clone();
            let decision = serde_json::to_vec(&serde_json::json!({
                "schema": 1, "intent_sha256": intent,
                "report_sha256": stutter_core::hash::sha256_hex(&report),
                "target_phase": phase, "terminal_failure": cause, "verdict": verdict,
                "protected_state_seal_sha256": seal_digest,
                "protected_state_seal_valid": seal_valid,
            }))?;
            write_identical(&dir, uid, "finalization-prepared.json", &decision)?;
            Ok(PreparedFinalization {
                dir,
                bytes: decision,
                phase,
                code,
                cause,
            })
        })();
        let PreparedFinalization {
            dir,
            bytes: prepared,
            phase,
            code,
            cause,
        } = match preparation {
            Ok(value) => value,
            Err(error) => {
                log::error!("research preterminal verification failed: {error:#}");
                self.storage_blocker = Some("UnsafeToResume");
                return;
            }
        };
        self.settle(now, phase, code, cause);
        if self.active.is_some() || self.terminal.is_none() {
            return;
        }
        let verified = self.verify_terminal_artifacts(&dir, uid, &prepared, phase, code);
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.report_persisted = Some(true);
            terminal.finalization_verified = Some(verified.is_ok());
            if let Err(error) = verified {
                log::error!("research terminal verification failed: {error:#}");
                terminal.code = "TerminalVerificationFailed";
                self.storage_blocker = Some("UnsafeToResume");
            }
        }
    }
    fn verify_terminal_artifacts(
        &self,
        dir: &File,
        uid: u32,
        prepared: &[u8],
        phase: ResearchPhase,
        code: &'static str,
    ) -> anyhow::Result<()> {
        let lease_dir = self
            .campaign_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("lease directory unavailable"))?;
        let lease = crate::autotune::research::lease_inspect::inspect(lease_dir, uid)?;
        let terminal = self
            .terminal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("terminal state unavailable"))?;
        anyhow::ensure!(
            lease.chain_valid
                && lease.transitions_valid
                && lease.header_matches_intent
                && lease.terminal
                && lease.final_phase == phase
                && lease.intent_sha256 == terminal.intent_sha256
                && lease.owner_live == Some(false),
            "terminal lease verification failed"
        );
        let mut response: serde_json::Value =
            serde_json::from_str(&self.terminal_finalization_evidence(uid))?;
        anyhow::ensure!(
            response["reconstruction"] == "passed" && response["bundle"]["written"] == true,
            "terminal report or bundle unavailable"
        );
        let before: serde_json::Value = serde_json::from_slice(prepared)?;
        // The seal the prepared decision names is the one on disk now: nothing sealed or replaced it afterwards.
        let sealed_now = crate::autotune::research::protected_state::seal_digest(
            dir,
            uid,
            &terminal.intent_sha256,
        );
        anyhow::ensure!(
            before["protected_state_seal_sha256"] == serde_json::json!(sealed_now),
            "the protected-state seal changed after the prepared decision"
        );
        // A completed campaign names a valid seal: present, well-formed and bound to this campaign.
        anyhow::ensure!(
            phase != ResearchPhase::Completed
                || (sealed_now.is_some()
                    && before["protected_state_seal_valid"] == true
                    && crate::autotune::research::protected_state::sealed(
                        dir,
                        uid,
                        &terminal.intent_sha256
                    )
                    .is_ok()),
            "a completed campaign needs a valid protected-state seal"
        );
        anyhow::ensure!(
            before["intent_sha256"] == terminal.intent_sha256
                && before["target_phase"] == serde_json::to_value(phase)?,
            "prepared decision names another terminal state"
        );
        let earlier = before["verdict"]["gates"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("prepared gates missing"))?;
        let later = response["verdict"]["gates"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("terminal gates missing"))?;
        anyhow::ensure!(
            earlier
                .iter()
                .filter(|gate| gate["gate"] != "G1")
                .eq(later.iter().filter(|gate| gate["gate"] != "G1")),
            "scientific or restoration gates changed during release"
        );
        if phase == ResearchPhase::Completed {
            anyhow::ensure!(
                response["verdict"]["final_gate_qualified"] == true && response["status"] == "ok",
                "terminal qualification did not replay"
            );
        } else if code == "PartiallyQualified" {
            anyhow::ensure!(
                response["verdict"]["disposition"] == "PartiallyQualified",
                "terminal partial qualification did not replay"
            );
        }
        let bundle = response["bundle"]["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("bundle name missing"))?
            .to_owned();
        response
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("invalid verification response"))?
            .remove("bundle");
        let report_bytes =
            crate::autotune::research::report_store::read_private(dir, "report.json", uid)?;
        anyhow::ensure!(
            before["report_sha256"].as_str()
                == Some(stutter_core::hash::sha256_hex(&report_bytes).as_str()),
            "report changed during release"
        );
        let report: serde_json::Value = serde_json::from_slice(&report_bytes)?;
        crate::autotune::research::final_bundle::verify_written(
            dir, uid, &report, &response, &bundle,
        )?;
        let receipt = serde_json::to_vec(&serde_json::json!({
            "schema": 1, "intent_sha256": terminal.intent_sha256,
            "prepared_sha256": stutter_core::hash::sha256_hex(prepared),
            "terminal_phase": phase, "bundle": bundle,
            "verdict": response["verdict"],
        }))?;
        write_identical(dir, uid, "finalization-verified.json", &receipt)?;
        if phase == ResearchPhase::Completed {
            let typed = serde_json::from_value(report)?;
            verify_completed_receipt(dir, uid, &typed)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "research_control_finalize_tests.rs"]
mod tests;
