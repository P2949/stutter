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
        })
    }

    pub(super) fn report_response(&self, verify: bool, uid: u32) -> String {
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
                // verification status stands on its own evidence (see `verdict`); the response
                // stays `IncompleteProvenance` until the final gate is genuinely qualified.
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
                let verdict = crate::autotune::research::verdict::judge(
                    &crate::autotune::research::verdict::VerdictInputs {
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
                        child_replication: children.as_ref().and_then(|c| c.holdout_replication()),
                        // G8 stands on a receipt of what is deployed, about the very binary
                        // doing this verification; none is inferred from the Nix pin.
                        lease: self.lease_facts(&report.intent_sha256, uid),
                        host: intent.as_ref().zip(self.steamapps_dir.as_deref()).and_then(
                            |(intent, steamapps)| {
                                crate::autotune::research::host_baseline::inspect(
                                    intent, steamapps, uid,
                                )
                            },
                        ),
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
                                    )
                                })
                        }),
                        // Restoration is established only by independent campaign evidence: every
                        // planned action study qualified with valid kernel-state receipts, and no
                        // research prefix still carrying an unrestored settings edit. Unknown is false.
                        restoration_verified: restored,
                        health_blocked,
                    },
                );
                let mut body = serde_json::json!({
                    "status": if verdict.final_gate_qualified { "ok" } else { "blocked" },
                    "code": if verdict.final_gate_qualified { "Qualified" } else { "IncompleteProvenance" },
                    "reconstruction": "passed",
                    "verdict": verdict,
                    "raw_native_files": raw,
                    "child_studies": studies,
                    "child_reports": children,
                    "intent_sha256": report.intent_sha256,
                });
                // The named final artifacts, once, beside the report. A verdict whose bundle could
                // not be made durable is not a final one: the response says so and is blocked.
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
