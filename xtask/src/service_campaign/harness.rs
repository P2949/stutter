//! Live stage execution; failures remain failures until their full evidence requirements pass.
use stutter_validation_runner::model::{BoundedServiceAction, Request};

use super::{
    driver::{StageRunner, write_report},
    model::{Stage, State},
    runner,
};
use crate::validation_campaign::{model::AttemptOutcome, state::CampaignDir};
pub struct Production;
impl StageRunner for Production {
    fn after_persist(
        &self,
        stage: Stage,
        state: &State,
        outcome: &AttemptOutcome,
    ) -> anyhow::Result<()> {
        if stage == Stage::Reboots && *outcome == AttemptOutcome::WaitingForReboot {
            runner::request(state, stage, Request::Reboot)?;
            // The runner reboots asynchronously; this process must not outrun it.
            std::thread::sleep(std::time::Duration::from_secs(120));
            anyhow::bail!("the typed reboot was accepted but the machine did not go down");
        }
        Ok(())
    }

    fn execute(
        &self,
        stage: Stage,
        dir: &CampaignDir,
        state: &mut State,
        attempt: u32,
    ) -> anyhow::Result<AttemptOutcome> {
        match stage {
            Stage::Preflight => {
                let (outcome, report) = super::preflight::check(state)?;
                write_report(dir, stage, attempt, &report)?;
                Ok(outcome)
            }
            Stage::Install => {
                let report = runner::request(
                    state,
                    stage,
                    Request::BoundedService {
                        action: BoundedServiceAction::InstallPinned,
                    },
                )?;
                write_report(dir, stage, attempt, &report)?;
                Ok(AttemptOutcome::Passed)
            }
            Stage::Discovery => {
                let output = dir.attempt_dir(stage.id(), attempt).join("live");
                let report = crate::soak::service_discovery::run(state, &output)?;
                write_report(dir, stage, attempt, &report)?;
                Ok(AttemptOutcome::Passed)
            }
            Stage::Soak => {
                let output = dir.0.join("service-soak");
                let report = crate::soak::service_soak::campaign_run(state, &output)?;
                write_report(dir, stage, attempt, &serde_json::to_value(&report)?)?;
                Ok(if report.passed || state.rehearsal {
                    AttemptOutcome::Passed
                } else {
                    AttemptOutcome::Failed { class: crate::validation_campaign::model::FailureClass::ValidationFailure,
                        stage: stage.id().into(), summary: "packaged service soak did not meet its frozen coverage and dwell requirements".into() }
                })
            }
            Stage::Recovery => {
                let output = dir.attempt_dir(stage.id(), attempt).join("live");
                let report = crate::soak::service_recovery::run(state, &output)?;
                write_report(dir, stage, attempt, &report)?;
                Ok(AttemptOutcome::Passed)
            }
            // These stages cannot pass on a template/source check. Their live implementations
            // must produce the complete reboot and evidence records before promotion.
            Stage::Reboots => {
                let outcome = crate::soak::service_reboots::step(state, dir, attempt)?;
                if outcome == AttemptOutcome::Passed {
                    write_report(
                        dir,
                        stage,
                        attempt,
                        &serde_json::json!({"passed": true, "boot_ids": state.boot_ids}),
                    )?;
                }
                Ok(outcome)
            }
            Stage::Evidence => {
                anyhow::ensure!(!state.rehearsal, "a timing rehearsal is never evidence");
                let report = crate::soak::service_evidence::run(state, dir, attempt)?;
                write_report(dir, stage, attempt, &report)?;
                Ok(AttemptOutcome::Passed)
            }
        }
    }
}
