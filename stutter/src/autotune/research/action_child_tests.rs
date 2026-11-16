use super::*;

const TARGET: ChildTarget = ChildTarget {
    pid: stutter_core::ids::Pid::new(4242),
    start_ticks: 99,
    uid: 1000,
};
const NOW: u64 = 1_000;

fn scope() -> ActionChildScope {
    ActionChildScope::new(
        [
            ResearchAction::CpuAffinityProfile,
            ResearchAction::CgroupPlacement,
        ],
        TARGET,
        NOW + 60,
    )
}

fn ask(action: ResearchAction, target: ChildTarget, now: u64) -> ActionRequest {
    ActionRequest {
        action,
        target,
        now_unix: now,
    }
}

#[test]
fn only_a_named_action_on_the_frozen_process_inside_the_window_is_authorized() {
    let mut narrow = ActionChildScope::new([ResearchAction::CpuAffinityProfile], TARGET, NOW + 60);
    assert_eq!(
        narrow
            .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
            .unwrap_err(),
        Refusal::ActionNotAuthorized
    );
    for wrong in [
        ChildTarget {
            pid: stutter_core::ids::Pid::new(4243),
            ..TARGET
        },
        ChildTarget {
            start_ticks: 100,
            ..TARGET
        },
        ChildTarget { uid: 0, ..TARGET },
    ] {
        assert_eq!(
            scope()
                .authorize(ask(ResearchAction::CpuAffinityProfile, wrong, NOW))
                .unwrap_err(),
            Refusal::WrongTarget,
            "a reused pid, a restarted process or another owner is not the game"
        );
    }
    assert_eq!(
        scope()
            .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW + 61))
            .unwrap_err(),
        Refusal::Expired
    );
    // Refusals change nothing: the scope still grants the valid request.
    let mut fresh = scope();
    assert_eq!(
        fresh
            .authorize(ask(
                ResearchAction::CpuAffinityProfile,
                ChildTarget {
                    pid: stutter_core::ids::Pid::new(1),
                    ..TARGET
                },
                NOW
            ))
            .unwrap_err(),
        Refusal::WrongTarget
    );
    assert!(
        fresh
            .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
            .is_ok()
    );
}

#[test]
fn one_action_at_a_time_and_never_a_retry_of_the_same_action() {
    let mut scope = scope();
    let grant = scope
        .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
        .unwrap();
    // While it is granted, applied, kept or reverting, nothing else may start.
    assert_eq!(
        scope
            .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
            .unwrap_err(),
        Refusal::ActionOutstanding
    );
    assert!(scope.applied(grant, true));
    assert!(!scope.is_clean());
    assert_eq!(
        scope
            .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
            .unwrap_err(),
        Refusal::ActionOutstanding
    );
    assert!(scope.decide(Decision::Revert));
    assert_eq!(
        scope
            .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
            .unwrap_err(),
        Refusal::ActionOutstanding,
        "restoration is not yet proven"
    );
    assert!(
        !scope.restored(false),
        "an unproven restoration frees nothing"
    );
    assert!(!scope.is_clean());
    assert!(scope.restored(true));
    assert!(scope.is_clean());
    // The reverted action is not tried again; the other one may now run.
    assert_eq!(
        scope
            .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
            .unwrap_err(),
        Refusal::AlreadyAttempted
    );
    assert!(
        scope
            .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
            .is_ok()
    );
}

#[test]
fn a_failed_readback_can_only_be_undone_and_a_kept_action_stays_outstanding_until_restored() {
    let mut failed = scope();
    let grant = failed
        .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
        .unwrap();
    assert!(failed.applied(grant, false));
    assert!(
        !failed.decide(Decision::Keep),
        "an unverified action cannot be kept"
    );
    assert!(
        !failed.decide(Decision::Revert),
        "it is already being undone"
    );
    assert!(failed.restored(true));
    assert_eq!(
        failed.settled(),
        &[(ResearchAction::CpuAffinityProfile, Decision::Revert)]
    );

    let mut kept = scope();
    let grant = kept
        .authorize(ask(ResearchAction::CgroupPlacement, TARGET, NOW))
        .unwrap();
    assert!(kept.applied(grant, true));
    assert!(kept.decide(Decision::Keep));
    assert!(!kept.is_clean(), "a kept action is still a live mutation");
    assert_eq!(
        kept.authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
            .unwrap_err(),
        Refusal::ActionOutstanding
    );
    assert!(kept.restored(true));
    assert!(kept.is_clean());
    assert_eq!(
        kept.settled(),
        &[(ResearchAction::CgroupPlacement, Decision::Keep)]
    );
}

#[test]
fn a_worker_that_never_applied_leaves_nothing_to_restore_and_phases_cannot_be_skipped() {
    let mut scope = scope();
    let grant = scope
        .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
        .unwrap();
    // Deciding or restoring before anything was applied is refused.
    assert!(!scope.decide(Decision::Keep));
    assert!(!scope.restored(true));
    assert!(scope.not_applied(grant));
    assert!(scope.is_clean());
    assert!(scope.settled().is_empty());
    // The refused action still counts as attempted.
    assert_eq!(
        scope
            .authorize(ask(ResearchAction::CpuAffinityProfile, TARGET, NOW))
            .unwrap_err(),
        Refusal::AlreadyAttempted
    );
}
