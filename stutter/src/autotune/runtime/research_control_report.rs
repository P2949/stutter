//! Report and verification responses of a resident research campaign (read-only over the
//! campaign directory; the final bundle is derived from what a verification established).
use std::fs::File;

use super::ResearchCoordinator;

impl ResearchCoordinator {
    /// Replay the sealed native result files of the campaign in `dir` under each arm's own prefix.
    fn raw_replay(
        &self,
        dir: &File,
        uid: u32,
    ) -> (
        serde_json::Value,
        Option<crate::autotune::research::raw_verify::RawVerification>,
    ) {
        use std::os::fd::AsFd;

        use crate::autotune::research::{
            ResearchIntentV1, ResearchJournal, driver::build_schedule, raw_verify,
            report_store::read_private, session_host,
        };
        let replay = || -> anyhow::Result<raw_verify::RawVerification> {
            let study = self
                .study_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no study directory"))?;
            let intent: ResearchIntentV1 =
                serde_json::from_slice(&read_private(dir, "intent.json", uid)?)?;
            let journal = ResearchJournal::open(
                crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
                uid,
                build_schedule(&intent)?,
            )?;
            let records = journal.verify()?;
            let evidence = File::from(crate::syscall::openat_dir_nofollow(
                dir.as_fd(),
                "evidence",
            )?);
            raw_verify::verify_raw_evidence(&evidence, uid, &records, &|arm| {
                session_host::arm_prefix_paths(study, arm).1
            })
        };
        match replay() {
            Ok(outcome) => (
                serde_json::json!({
                    "status": if outcome.passed() { "replayed" } else { "failed" },
                    "detail": outcome,
                }),
                Some(outcome),
            ),
            Err(error) => (
                serde_json::json!({ "status": "error", "reason": format!("{error:#}") }),
                None,
            ),
        }
    }

    /// What a read-only inspection of this campaign's lease directory found (G1); `None` when there
    /// is no lease directory or it cannot be inspected.
    fn lease_facts(
        &self,
        intent_sha256: &str,
        uid: u32,
    ) -> Option<crate::autotune::research::gate_verdict::LeaseFacts> {
        let dir = self.campaign_dir.as_deref()?;
        let inspection = crate::autotune::research::lease_inspect::inspect(dir, uid).ok()?;
        Some(crate::autotune::research::gate_verdict::LeaseFacts {
            chain_valid: inspection.chain_valid,
            transitions_valid: inspection.transitions_valid,
            header_matches_intent: inspection.header_matches_intent,
            terminal: inspection.terminal,
            for_this_campaign: inspection.intent_sha256 == intent_sha256,
            owner_live: inspection.owner_live,
            admission: inspection.admission,
        })
    }

    pub(super) fn report_response(&self, verify: bool, uid: u32) -> String {
        self.report_response_inner(verify, uid, false, false)
    }

    /// Read-only evidence reconstruction while the coordinator still holds the restoring lease.
    pub(super) fn finalization_evidence(&self, uid: u32) -> String {
        if self.driver.is_some()
            || !self.active.as_ref().is_some_and(|active| {
                active.lease.phase() == crate::autotune::research::ResearchPhase::Restoring
            })
        {
            return super::response("blocked", "FinalizationOwnershipUnproven");
        }
        self.report_response_inner(true, uid, true, false)
    }

    /// The first post-release reconstruction creates the receipt that ordinary verification requires.
    pub(super) fn terminal_finalization_evidence(&self, uid: u32) -> String {
        self.report_response_inner(true, uid, false, true)
    }

    fn report_response_inner(
        &self,
        verify: bool,
        uid: u32,
        preparing: bool,
        finalizing: bool,
    ) -> String {
        let Some(dir) = self.open_campaign_study_dir() else {
            return super::response(
                "blocked",
                if verify {
                    "NoReportToVerify"
                } else {
                    "NoReport"
                },
            );
        };
        match crate::autotune::research::report_store::load_and_verify(&dir, uid) {
            Ok(report) if verify => {
                // The raw native files the sealed records name are reopened and re-hashed too. Each
                // verification status stands on its own evidence (see `verdict`).
                let (raw, raw_outcome) = self.raw_replay(&dir, uid);
                let intent =
                    crate::autotune::research::report_store::read_private(&dir, "intent.json", uid)
                        .ok()
                        .and_then(|bytes| {
                            serde_json::from_slice::<crate::autotune::research::ResearchIntentV1>(
                                &bytes,
                            )
                            .ok()
                        });
                let studies = intent.as_ref().and_then(|intent| {
                    crate::autotune::research::child_ledger::read_dispositions(&dir, uid, intent)
                        .ok()
                });
                // Each child study's own report and raw files, verified like the campaign's.
                let children =
                    intent
                        .as_ref()
                        .zip(self.study_dir.as_deref())
                        .and_then(|(intent, study)| {
                            crate::autotune::research::child_verify::verify_children(
                                &dir,
                                uid,
                                intent,
                                &|arm| {
                                    crate::autotune::research::session_host::arm_prefix_paths(
                                        study, arm,
                                    )
                                    .1
                                },
                            )
                            .ok()
                        });
                let evidence: Vec<_> = {
                    use std::os::fd::AsFd;
                    crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence")
                        .ok()
                        .map(File::from)
                        .and_then(|evidence_dir| {
                            crate::autotune::research::report_store::read_evidence(
                                &evidence_dir,
                                uid,
                            )
                            .ok()
                        })
                        .map(|items| items.into_values().collect())
                        .unwrap_or_default()
                };
                let health_blocked = self.study_dir.as_deref().is_none_or(|study| {
                    crate::autotune::research::gpu_health::state_blocked(study)
                });
                let restored = children
                    .as_ref()
                    .zip(studies.as_deref())
                    .is_some_and(|(children, planned)| children.actions_restored(planned))
                    && intent.as_ref().zip(self.study_dir.as_deref()).is_some_and(
                        |(intent, study)| {
                            intent.tools.iter().all(|tool| {
                                !crate::autotune::research::profile_applier::pending_edit(
                                    &crate::autotune::research::session_host::arm_prefix_paths(
                                        study,
                                        tool.source,
                                    )
                                    .2,
                                )
                            })
                        },
                    );
                let inputs = crate::autotune::research::verdict::VerdictInputs {
                        reconstruction_passed: true,
                        raw: raw_outcome.as_ref(),
                        evidence: &evidence,
                        child_studies: studies.as_deref().unwrap_or(&[]),
                        child_reports_verified: children
                            .as_ref()
                            .is_some_and(|children| children.all_verified()),
                        funnel: children
                            .as_ref()
                            .is_some_and(|c| !c.children.is_empty() || !c.missing.is_empty()),
                        child_runtime_qualified: children
                            .as_ref()
                            .is_some_and(|c| c.runtime_qualified()),
                        child_telemetry: children.as_ref().and_then(|c| c.telemetry_summary()),
                        // G3 stands on the resident runtime's own observation and on the preregistered oracle
                        // controls (two distinct causes, the GPU-bound negative, no unsafe action), both mandatory.
                        sealed_protected_state: match (
                            self.host_sources.0.as_deref(),
                            self.study_dir
                                .as_deref()
                                .and_then(|study| std::fs::File::open(study).ok()),
                        ) {
                            (Some(steamapps), Some(study)) => {
                                // The first evaluation that is not still preparing, with the saves unchanged, seals
                                // that fact once; later changes cannot rewrite it.
                                if !preparing {
                                    let _ = crate::autotune::research::protected_state::seal_unchanged(
                                        steamapps,
                                        &study,
                                        uid,
                                        &report.intent_sha256,
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map_or(0, |elapsed| elapsed.as_secs()),
                                    );
                                }
                                crate::autotune::research::protected_state::sealed(
                                    &study,
                                    uid,
                                    &report.intent_sha256,
                                )
                            }
                            _ => Err("ProtectedStateNotSealedAtFinalization"),
                        },
                        child_resident: children
                            .as_ref()
                            .map_or(Err("ResidentObservationNotCollected"), |c| c.resident_summary())
                            .and_then(|()| {
                                match self.study_dir.as_deref().map_or(
                                    Err("NoOracleCases"),
                                    crate::autotune::research::oracle_cases::evaluate_study,
                                ) {
                                    Ok(evaluation) if evaluation.gate_passed => Ok(()),
                                    Ok(_) => Err("OracleGateNotPassed"),
                                    Err(reason) => Err(reason),
                                }
                            }),
                        child_replication: children.as_ref().zip(studies.as_deref())
                            .and_then(|(children, planned)| children.holdout_replication(planned)),
                        control_socket: self.host_sources.2.as_deref().and_then(|dir| {
                            crate::autotune::research::control_socket::inspect(dir, uid)
                        }),
                        hardware: self
                            .study_dir
                            .as_deref()
                            .and_then(|study| std::fs::File::open(study).ok())
                            .map_or(Err("HostHardwareNotRecorded"), |study| {
                                crate::autotune::research::host_hardware::status(
                                    &crate::autotune::research::host_hardware::Roots::system(),
                                    &study,
                                    uid,
                                    &report.intent_sha256,
                                )
                            }),
                        protected_state: match (
                            self.host_sources.0.as_deref(),
                            self.study_dir
                                .as_deref()
                                .and_then(|study| std::fs::File::open(study).ok()),
                        ) {
                            (Some(steamapps), Some(study)) => {
                                crate::autotune::research::protected_state::status(
                                    steamapps, &study, uid, &report.intent_sha256,
                                )
                            }
                            _ => Err("ProtectedStateNotRecorded"),
                        },
                        fault_matrix: match (
                            self.study_dir.as_deref(),
                            crate::autotune::research::deploy_receipt::own_binary_sha256(),
                        ) {
                            (Some(study), Some(own)) => {
                                crate::autotune::research::fault_matrix::gate_status(
                                    study,
                                    &own,
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map_or(0, |elapsed| elapsed.as_secs()),
                                )
                            }
                            _ => Err("NoFaultMatrixReceipt"),
                        },
                        pressure_control: match (
                            self.study_dir.as_deref(),
                            crate::autotune::research::deploy_receipt::own_binary_sha256(),
                        ) {
                            (Some(study), Some(own)) => {
                                crate::autotune::research::pressure_control::status(
                                    study,
                                    &own,
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map_or(0, |elapsed| elapsed.as_secs()),
                                )
                            }
                            _ => Err("NoPressureControl"),
                        },
                        // G8 stands on a receipt of what is deployed, about the very binary
                        // doing this verification; none is inferred from the Nix pin.
                        lease: self.lease_facts(&report.intent_sha256, uid),
                        host: intent
                            .as_ref()
                            .zip(self.host_sources.0.as_deref())
                            .and_then(|(intent, steamapps)| {
                                crate::autotune::research::host_baseline::inspect(
                                    intent,
                                    steamapps,
                                    self.host_sources.1.as_deref(),
                                    uid,
                                )
                            }),
                        deployment_attested: self.study_dir.as_deref().is_some_and(|study| {
                            crate::autotune::research::deploy_receipt::own_binary_sha256()
                                .is_some_and(|own| {
                                    crate::autotune::research::deploy_receipt::deployment_attested(
                                        study,
                                        &own,
                                        stutter_core::boot::current_boot_id().ok().as_deref(),
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map_or(0, |elapsed| elapsed.as_secs()),
                                        intent.as_ref().map(|intent| {
                                            crate::autotune::research::deploy_receipt::CampaignBinding::for_this_build(
                                                &intent.source_sha256,
                                            )
                                        }),
                                    )
                                })
                        }),
                        // Restoration is established only by independent campaign evidence: every
                        // planned action study qualified with valid kernel-state receipts, and no
                        // research prefix still carrying an unrestored settings edit. Unknown is false.
                        restoration_verified: restored,
                        health_blocked,
                    };
                let verdict = if preparing {
                    crate::autotune::research::verdict::judge_preparing(&inputs)
                } else {
                    crate::autotune::research::verdict::judge(&inputs)
                };
                let mut body = serde_json::json!({
                    "status": if verdict.final_gate_qualified { "ok" } else { "blocked" },
                    "code": if verdict.final_gate_qualified { "QualifiedEndToEnd" } else if verdict.disposition == "AbortedHealth" { "AbortedHealth" } else if verdict.disposition == "PartiallyQualified" { "PartiallyQualified" } else { "BlockedCampaignOrStats" },
                    "reconstruction": "passed",
                    "verdict": verdict,
                    "raw_native_files": raw,
                    "child_studies": studies,
                    "child_reports": children,
                    "intent_sha256": report.intent_sha256,
                    // The diagnosis oracle's own scored cases (frozen truth, captured output), or
                    // exactly why there are none.
                    "diagnosis_oracle": match self.study_dir.as_deref().map_or(
                        Err("NoOracleCases"),
                        crate::autotune::research::oracle_cases::evaluate_study,
                    ) {
                        Ok(evaluation) => serde_json::json!({
                            "status": "scored", "evaluation": evaluation }),
                        Err(reason) => serde_json::json!({
                            "status": "not_scored", "reason": reason }),
                    },
                });
                if !preparing
                    && !finalizing
                    && self.terminal.as_ref().is_some_and(|terminal| {
                        terminal.phase == crate::autotune::research::ResearchPhase::Completed
                    })
                {
                    if let Err(error) =
                        super::finalize::verify_completed_receipt(&dir, uid, &report)
                    {
                        log::warn!("completed campaign receipt rejected: {error:#}");
                        body["status"] = "blocked".into();
                        body["code"] = "TerminalVerificationFailed".into();
                        body["historical_finalization_verified"] = false.into();
                        return body.to_string() + "\n";
                    }
                    body["historical_finalization_verified"] = true.into();
                }
                // The named final artifacts, once, beside the report. A verdict whose bundle could
                // not be made durable is not a final one: the response says so and is blocked.
                if preparing || self.active.is_some() {
                    return body.to_string() + "\n";
                }
                match crate::autotune::research::final_bundle::write_reporting(&dir, &report, &body)
                {
                    Ok(name) => {
                        body["bundle"] = serde_json::json!({ "written": true, "name": name })
                    }
                    Err(error) => {
                        body["status"] = "blocked".into();
                        body["code"] = "BundleNotWritten".into();
                        body["bundle"] = serde_json::json!({ "written": false, "error": error });
                    }
                }
                body.to_string() + "\n"
            }
            Ok(report) => {
                serde_json::json!({ "status": "ok", "code": "Report", "report": report })
                    .to_string()
                    + "\n"
            }
            Err(error) => {
                log::warn!("research report unavailable: {error:#}");
                super::response(
                    "blocked",
                    if verify {
                        "VerificationFailed"
                    } else {
                        "NoReport"
                    },
                )
            }
        }
    }
}
