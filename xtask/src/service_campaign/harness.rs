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
            // These stages cannot pass on a template/source check. Their live implementations
            // must produce the complete focus/fault/soak/reboot/evidence records before promotion.
            Stage::Recovery | Stage::Reboots | Stage::Soak | Stage::Evidence => {
                anyhow::bail!(
                    "service {} requires its complete live evidence runner",
                    stage.id()
                )
            }
        }
    }
}
