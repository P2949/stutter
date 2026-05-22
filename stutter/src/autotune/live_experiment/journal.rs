use std::path::{Path, PathBuf};

use super::*;
use crate::autotune::{
    candidate::CandidateAction,
    controller_journal::{
        ControllerJournalActionMetadata, ControllerJournalRecord, ControllerJournalState,
        default_controller_journal_path, journal_process_identity, write_controller_journal_record,
    },
    observation::{ActiveConfigSnapshot, AutotuneObservation},
    system_context::{SystemContextSnapshotInput, collect_system_context},
};

pub(super) fn controller_journal_record_for_live_experiment(
    input: &LiveExperimentManagerInput<'_>,
    experiment: &LiveExperiment,
    observation: &AutotuneObservation,
    state: ControllerJournalState,
    verify_result: &str,
) -> ControllerJournalRecord {
    let action_id = experiment.action_id();
    ControllerJournalRecord::for_phase(
        state,
        experiment.experiment_id.as_str(),
        action_id,
        Some(experiment.rollback.clone()),
    )
    .with_metadata(controller_journal_metadata_for_candidate(
        input,
        &experiment.candidate,
        observation,
        Some(experiment.rollback.affected_tasks()),
        verify_result,
    ))
    .with_mode(experiment.mode)
    .with_safety_class(experiment.safety_class.clone())
}
pub(super) fn write_controller_journal_phase_for_live_experiment(
    input: &LiveExperimentManagerInput<'_>,
    experiment: &LiveExperiment,
    observation: &AutotuneObservation,
    state: ControllerJournalState,
    verify_result: &str,
) -> anyhow::Result<()> {
    if input.simulate_action_effects && input.controller_journal_path.is_none() {
        return Ok(());
    }

    let record = controller_journal_record_for_live_experiment(
        input,
        experiment,
        observation,
        state,
        verify_result,
    );
    write_controller_journal_record(&controller_journal_path(input), &record)
}
pub(super) fn controller_journal_metadata_for_candidate(
    input: &LiveExperimentManagerInput<'_>,
    candidate: &CandidateAction,
    observation: &AutotuneObservation,
    affected_tasks: Option<usize>,
    verify_result: &str,
) -> ControllerJournalActionMetadata {
    let pid = observation
        .target_root_pid
        .filter(|pid| *pid != 0)
        .unwrap_or_else(|| candidate.tree_pid());
    let starttime_ticks = (pid != 0)
        .then(|| crate::process_tree::process_starttime_at(Path::new("/proc"), pid))
        .flatten();
    let active_task_count = affected_tasks.or(Some(observation.active_target_count));

    ControllerJournalActionMetadata::default()
        .with_candidate(candidate.profile_name().to_owned())
        .with_workload_identity(journal_process_identity(pid, starttime_ticks, None))
        .with_target_identity(journal_process_identity(
            pid,
            starttime_ticks,
            active_task_count,
        ))
        .with_restore_command(input.manual_restore_command)
        .with_verify_result(verify_result)
        .with_mode(input.mode)
        .with_safety_class(candidate.safety_class())
}
pub(super) fn controller_journal_path(input: &LiveExperimentManagerInput<'_>) -> PathBuf {
    input
        .controller_journal_path
        .clone()
        .unwrap_or_else(default_controller_journal_path)
}
pub(super) fn collect_post_rollback_active_config(
    observation: &AutotuneObservation,
) -> Option<ActiveConfigSnapshot> {
    observation.active_config_snapshot.as_ref()?;

    Some(
        collect_system_context(SystemContextSnapshotInput {
            proc_root: Path::new("/proc"),
            sys_root: Path::new("/sys"),
            active_tasks: &observation.active_tasks,
            health: observation.system_health.clone(),
            sampled_at_unix_nanos: crate::audit::unix_nanos_now(),
        })
        .active_config,
    )
}
