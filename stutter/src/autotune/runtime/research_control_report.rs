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
                        // No physical restoration verifier exists yet; never inferred.
                        restoration_verified: false,
                        health_blocked,
                    },
                );
                let body = serde_json::json!({
                    "status": if verdict.final_gate_qualified { "ok" } else { "blocked" },
                    "code": if verdict.final_gate_qualified { "Qualified" } else { "IncompleteProvenance" },
                    "reconstruction": "passed",
                    "verdict": verdict,
                    "raw_native_files": raw,
                    "child_studies": studies,
                    "child_reports": children,
                    "intent_sha256": report.intent_sha256,
                });
                // The named final artifacts, once, beside the report; a failure is not a verdict.
                crate::autotune::research::final_bundle::write_logged(&dir, &report, &body);
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
