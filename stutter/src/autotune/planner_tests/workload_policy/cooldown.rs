use super::support::*;

#[test]
fn workload_memory_cools_down_same_workload_without_blocking_other_workload() {
    let policy = policy(DaemonMode::Suggest);
    let candidate = nice_candidate();
    let mut controller_state = ControllerRuntimeState::default();
    let mut same_workload = observation();
    same_workload.primary_situation = SituationKind::CompileCpuBound;
    same_workload.focus_kind = Some(FocusGroupKind::Compile);
    same_workload.refresh_situation_classification();
    controller_state.record_candidate_result(
        crate::autotune::controller::ControllerCandidateResultInput {
            candidate: &candidate,
            observation: &same_workload,
            cpu_topology_signature: None,
            result: CandidateMemoryResult::Reverted,
            diagnostic_baseline_raw_score_total: Some(100),
            diagnostic_current_raw_score_total: Some(120),
            rollback_reason: Some("regressed".to_owned()),
            cooldown_expires_unix_nanos: Some(same_workload.now_unix_nanos + 10_000),
        },
    );

    let mut dry_runner = CountingDryRunner::default();
    let same_eval = evaluate_candidate_with_runner(
        &policy,
        &same_workload,
        &same_workload.capabilities,
        &controller_state,
        candidate.clone(),
        1.0,
        &mut dry_runner,
    );
    assert!(
        same_eval
            .deny_reasons
            .contains(&CandidateDenyReason::CooldownActive)
    );

    let mut other_workload = same_workload.clone();
    other_workload
        .workload_identity
        .as_mut()
        .unwrap()
        .stable_hash = "different-workload".to_owned();
    other_workload.workload_identity.as_mut().unwrap().exe_ino = Some(99);
    let mut dry_runner = CountingDryRunner::default();
    let other_eval = evaluate_candidate_with_runner(
        &policy,
        &other_workload,
        &other_workload.capabilities,
        &controller_state,
        candidate,
        1.0,
        &mut dry_runner,
    );

    assert!(
        !other_eval
            .deny_reasons
            .contains(&CandidateDenyReason::CooldownActive)
    );
}
