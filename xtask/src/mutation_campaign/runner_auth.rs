//! request. Derived from the operation's manifest section; nothing is chosen by a prompt.
use std::{collections::BTreeSet, path::Path};

use stutter_validation_runner::model::{ControllerKind, LiveSuite, RunnerAction, TaskGrant};

use super::{graph::Stage, manifest::OperationSection, model::MutationState};
use crate::validation_campaign::bootstrap::{CAMPAIGN_TASK, RunnerTarget};

pub fn grants(operation: &OperationSection) -> BTreeSet<TaskGrant> {
    let mut grants = BTreeSet::new();
    for (stage, suite) in [
        (Stage::Mechanical, operation.suites.mechanical.as_deref()),
        (Stage::FaultMatrix, operation.suites.fault_matrix.as_deref()),
    ] {
        if let Some(live) = suite.and_then(LiveSuite::from_xtask_name) {
            grants.insert(TaskGrant {
                task_id: stage.as_str().to_owned(),
                action: RunnerAction::ExecuteLiveSuite,
                suite: Some(live),
            });
        }
    }
    // The benefit stage measures its pairs with the benefit suite, nothing else. The holdout
    // stage reads the stored pairs and needs no privilege.
    grants.insert(TaskGrant {
        task_id: Stage::BenefitCalibration.as_str().to_owned(),
        action: RunnerAction::ExecuteLiveSuite,
        suite: Some(LiveSuite::AffinityBenefit),
    });
    // Its controlled scheduler contention is requested as a typed burst, never started by the
    // suite itself with privilege.
    grants.insert(TaskGrant {
        task_id: Stage::BenefitCalibration.as_str().to_owned(),
        action: RunnerAction::ExecuteSchedulerBurst,
        suite: None,
    });
    for action in [
        RunnerAction::Status,
        RunnerAction::ResumeCampaign,
        RunnerAction::CancelCampaign,
    ] {
        grants.insert(TaskGrant {
            task_id: CAMPAIGN_TASK.to_owned(),
            action,
            suite: None,
        });
    }
    grants
}

pub fn target(
    state: &MutationState,
    operation: &OperationSection,
    repository: &Path,
) -> RunnerTarget {
    RunnerTarget {
        campaign_id: state.campaign_id.clone(),
        uid: state.uid,
        gid: state.gid,
        repository: repository.to_path_buf(),
        manifest_sha256: state.manifest_sha256.clone(),
        xtask_sha256: state.xtask_sha256.clone(),
        stutter_sha256: state.stutter_sha256.clone(),
        controller: ControllerKind::MutationGraduation,
        grants: grants(operation),
        // A mutation campaign never reboots the machine through the runner on its own.
        max_reboots: 0,
        reboot_slack: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutation_campaign::manifest::MutationManifest;

    fn affinity() -> OperationSection {
        MutationManifest::parse(include_str!(
            "../../../docs/validation/mutation-graduation.toml"
        ))
        .unwrap()
        .operation("cpu_affinity_profile")
        .unwrap()
        .clone()
    }

    #[test]
    fn suites_are_granted_to_their_own_stage_only() {
        let grants = grants(&affinity());
        let live: Vec<_> = grants
            .iter()
            .filter(|g| g.action == RunnerAction::ExecuteLiveSuite)
            .collect();
        assert_eq!(live.len(), 3);
        assert!(grants.contains(&TaskGrant {
            task_id: "benefit_calibration".to_owned(),
            action: RunnerAction::ExecuteLiveSuite,
            suite: Some(LiveSuite::AffinityBenefit),
        }));
        assert!(grants.contains(&TaskGrant {
            task_id: "mechanical_apply_verify_rollback".to_owned(),
            action: RunnerAction::ExecuteLiveSuite,
            suite: Some(LiveSuite::Affinity),
        }));
        assert!(grants.contains(&TaskGrant {
            task_id: "fault_matrix".to_owned(),
            action: RunnerAction::ExecuteLiveSuite,
            suite: Some(LiveSuite::Recovery),
        }));
    }

    #[test]
    fn a_mutation_campaign_can_never_request_a_reboot_or_a_boot_change() {
        for grant in grants(&affinity()) {
            assert!(
                !matches!(
                    grant.action,
                    RunnerAction::Reboot
                        | RunnerAction::SetNextBoot
                        | RunnerAction::PrepareKernelCycle
                ),
                "{grant:?}"
            );
        }
    }
}
