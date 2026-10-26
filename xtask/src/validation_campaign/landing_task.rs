//! Shared plumbing for tasks that change the repository: apply the change on a campaign branch,
//! run the offline gates there, land it locally, and map every failure to the campaign's failure
//! classes.
use std::path::Path;

use super::{
    executor::{RunContext, RunnerOutcome},
    graph::TaskSpec,
    landing::{LandError, Landed, LocalChange, land},
    model::{BlockReason, FailureClass},
    runners::{OFFLINE_GATES, run_gates},
    shell::{Shell, SystemShell},
};

pub struct LandingTools {
    pub shell: Box<dyn Shell>,
    pub gates: Vec<(String, Vec<String>)>,
}

impl LandingTools {
    pub fn system() -> Self {
        Self {
            shell: Box::new(SystemShell),
            gates: OFFLINE_GATES
                .iter()
                .map(|(name, argv)| {
                    (
                        (*name).to_owned(),
                        argv.iter().map(|a| (*a).to_owned()).collect(),
                    )
                })
                .collect(),
        }
    }

    /// Branch name that is stable per campaign and task, so an interrupted flow resumes.
    pub fn branch(context: &RunContext<'_>, spec: &TaskSpec) -> String {
        format!("campaign/{}/{}", context.state.campaign_id, spec.id)
    }

    /// Applies `apply` on the branch, gates it, lands it. `Err` is the task outcome to record.
    pub fn land(
        &self,
        context: &RunContext<'_>,
        spec: &TaskSpec,
        attempt: u32,
        message: &str,
        apply: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<Landed, RunnerOutcome> {
        let branch = Self::branch(context, spec);
        let attempt_dir = context.dir.attempt_dir(spec.id, attempt);
        let artifacts = Some(context.dir.relative(&attempt_dir));
        let change = LocalChange {
            branch: &branch,
            commit_message: message,
        };
        let repository = context.repository;
        let apply_change = || apply(repository);
        let verify = || {
            run_gates(&self.gates, repository, &attempt_dir)
                .map_err(|failure| format!("{}: {}", failure.stage, failure.summary))
        };
        land(
            self.shell.as_ref(),
            repository,
            &change,
            &apply_change,
            &verify,
        )
        .map_err(|error| match error {
            LandError::Blocked(detail) => RunnerOutcome::Blocked(BlockReason {
                class: FailureClass::BlockedResource,
                detail,
            }),
            LandError::Failed {
                stage,
                detail,
                retryable,
            } => RunnerOutcome::Failed {
                class: if retryable {
                    FailureClass::Retryable
                } else {
                    FailureClass::SoftwareDefect
                },
                stage,
                summary: detail,
                artifacts,
            },
        })
    }
}
