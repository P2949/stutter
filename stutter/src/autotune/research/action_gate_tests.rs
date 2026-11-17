use super::*;
use crate::autotune::research::action_child::ActionChildScope;

const CPU: ResearchAction = ResearchAction::CpuAffinityProfile;
const CGROUP: ResearchAction = ResearchAction::CgroupPlacement;
const NOW: u64 = 1_000;

fn target() -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: 1000,
    }
}

fn gate() -> ActionGate {
    ActionGate::new(
        ActionChildScope::new([CPU, CGROUP], target(), NOW + 60),
        Box::new(|| None),
    )
}

fn view(active: bool, kept: usize, tokens: Option<u32>) -> GateView {
    // Whatever is held is the permitted CPU action of the frozen process, in one experiment.
    let holdings = (0..usize::from(active) + kept)
        .map(|_| Holding {
            experiment_id: stutter_core::ids::ExperimentId::new("exp-1"),
            action: Some(CPU),
            pid: Some(stutter_core::ids::Pid::new(4242)),
        })
        .collect();
    GateView {
        holdings,
        experiment_active: active,
        kept_actions: kept,
        outstanding_tokens: tokens,
    }
}

#[test]
fn only_an_authorized_action_on_the_frozen_process_is_permitted() {
    let mut g = gate();
    assert!(!g.permit(None, target(), NOW), "not a game action");
    let wrong = ChildTarget { uid: 0, ..target() };
    assert!(!g.permit(Some(CPU), wrong, NOW), "not the frozen process");
    assert!(
        !g.permit(Some(CPU), target(), NOW + 61),
        "outside the window"
    );
    assert!(g.permit(Some(CPU), target(), NOW));
    // One at a time: a second candidate is refused while the first is outstanding.
    assert!(!g.permit(Some(CGROUP), target(), NOW));
}

#[test]
fn a_reverted_action_is_settled_only_when_the_worker_holds_no_token() {
    let mut g = gate();
    assert!(g.permit(Some(CPU), target(), NOW));
    g.observe(view(true, 0, Some(1))); // applied, under measurement
    assert!(!g.evidence().clean);
    g.observe(view(true, 0, Some(1)));
    // The controller rolled it back, but the worker still holds a token: not yet restored.
    g.observe(view(false, 0, Some(1)));
    assert!(!g.evidence().clean && g.evidence().settled.is_empty());
    g.observe(view(false, 0, None));
    assert!(!g.evidence().clean, "an unknown token count is not zero");
    g.observe(view(false, 0, Some(0)));
    assert!(g.evidence().clean);
    assert_eq!(g.evidence().settled, [(CPU, Decision::Revert)]);
    // The same action is never tried again; the other one may now run.
    assert!(!g.permit(Some(CPU), target(), NOW));
    assert!(g.permit(Some(CGROUP), target(), NOW));
}

#[test]
fn a_kept_action_stays_outstanding_until_the_controller_releases_it_and_the_worker_is_clear() {
    let mut g = gate();
    assert!(g.permit(Some(CPU), target(), NOW));
    g.observe(view(true, 0, Some(1)));
    g.observe(view(false, 1, Some(1))); // kept
    assert!(!g.evidence().clean);
    assert!(
        !g.permit(Some(CGROUP), target(), NOW),
        "a kept action is still a live mutation"
    );
    g.observe(view(false, 1, Some(1)));
    g.observe(view(false, 0, Some(0))); // released and restored
    assert_eq!(g.evidence().settled, [(CPU, Decision::Keep)]);
    assert!(g.evidence().clean);
}

#[test]
fn a_candidate_the_controller_never_applied_changes_nothing_and_counts_as_attempted() {
    let mut g = gate();
    assert!(g.permit(Some(CPU), target(), NOW));
    // The event ended with no experiment and nothing kept: the controller declined or failed.
    g.observe(view(false, 0, Some(0)));
    assert!(g.evidence().clean && g.evidence().settled.is_empty());
    assert!(
        !g.permit(Some(CPU), target(), NOW),
        "the refused action is not retried"
    );
}

#[test]
fn the_process_identity_comes_from_proc_and_a_missing_process_has_none() {
    let me = process_identity(std::process::id()).expect("own identity");
    assert_eq!(me.pid, stutter_core::ids::Pid::new(std::process::id()));
    assert!(me.start_ticks > 0);
    assert_eq!(me.uid, crate::syscall::geteuid());
    // Reading again gives the same start time: it identifies this process, not a reused pid.
    assert_eq!(process_identity(std::process::id()), Some(me));
    assert!(process_identity(u32::MAX - 7).is_none());
}

#[test]
fn the_runtime_reports_only_experiment_state_and_the_gate_reads_the_workers_tokens_itself() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let tokens = Arc::new(AtomicU32::new(1));
    let probe = tokens.clone();
    let mut g = ActionGate::new(
        ActionChildScope::new([CPU], target(), NOW + 60),
        Box::new(move || Some(probe.load(Ordering::SeqCst))),
    );
    assert!(g.permit(Some(CPU), target(), NOW));
    assert!(g.is_busy());
    let own = vec![Holding {
        experiment_id: stutter_core::ids::ExperimentId::new("exp-1"),
        action: Some(CPU),
        pid: Some(stutter_core::ids::Pid::new(4242)),
    }];
    g.observe_holdings(true, 0, own);
    g.observe_runtime(false, 0); // rolled back while the worker still holds a token
    assert!(g.is_busy());
    tokens.store(0, Ordering::SeqCst);
    g.observe_runtime(false, 0);
    assert!(!g.is_busy());
    assert_eq!(g.evidence().settled, [(CPU, Decision::Revert)]);
}

mod kernel_state {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn state(cpus: &str, cgroup: &str) -> KernelState {
        KernelState {
            cpus_allowed: cpus.into(),
            cgroup: cgroup.into(),
            ..Default::default()
        }
    }

    /// A gate whose probe reports whatever the test last stored (`None`: unreadable).
    fn probed(shared: &Arc<Mutex<Option<KernelState>>>) -> ActionGate {
        let shared = shared.clone();
        gate().with_state_probe(Box::new(move |_| shared.lock().unwrap().clone()))
    }

    fn revert(g: &mut ActionGate, tokens: Option<u32>) {
        g.observe(view(true, 0, Some(1)));
        g.observe(view(false, 0, tokens));
    }

    #[test]
    fn a_zero_token_count_with_the_wrong_kernel_state_is_not_a_restoration() {
        let shared = Arc::new(Mutex::new(Some(state("0-15", "0::/app.slice"))));
        let mut g = probed(&shared);
        assert!(g.permit(Some(CPU), target(), NOW));
        // The action pinned the game; the controller rolled it back, the worker holds no token, but
        // the process is still pinned.
        *shared.lock().unwrap() = Some(state("2-3", "0::/app.slice"));
        revert(&mut g, Some(0));
        assert!(!g.evidence().clean && g.evidence().settled.is_empty());
        // Once the kernel really shows the original placement, it is accepted, with a receipt.
        *shared.lock().unwrap() = Some(state("0-15", "0::/app.slice"));
        g.observe(view(false, 0, Some(0)));
        let evidence = g.evidence();
        assert!(evidence.clean && evidence.kernel_state_proven);
        assert_eq!(evidence.receipts.len(), 1);
        assert_eq!(evidence.receipts[0].original, evidence.receipts[0].restored);
        assert_eq!(evidence.receipts[0].action, CPU);
    }

    #[test]
    fn a_changed_cgroup_or_an_unreadable_process_is_not_restored() {
        let shared = Arc::new(Mutex::new(Some(state("0-15", "0::/app.slice"))));
        let mut g = probed(&shared);
        assert!(g.permit(Some(CGROUP), target(), NOW));
        *shared.lock().unwrap() = Some(state("0-15", "0::/stutter.slice"));
        revert(&mut g, Some(0));
        assert!(!g.evidence().clean);
        *shared.lock().unwrap() = None;
        g.observe(view(false, 0, Some(0)));
        assert!(
            !g.evidence().clean,
            "a process that cannot be read is unproven"
        );
    }

    #[test]
    fn a_target_whose_state_cannot_be_read_is_never_permitted_an_action() {
        let shared = Arc::new(Mutex::new(None));
        let mut g = probed(&shared);
        assert!(!g.permit(Some(CPU), target(), NOW));
        assert!(!g.is_busy());
    }

    #[test]
    fn without_a_state_probe_the_kernel_state_is_never_claimed_proven() {
        let mut g = gate();
        assert!(g.permit(Some(CPU), target(), NOW));
        revert(&mut g, Some(0));
        let evidence = g.evidence();
        assert!(evidence.clean && !evidence.kernel_state_proven && evidence.receipts.is_empty());
    }
}

#[test]
fn the_real_proc_reader_sees_this_process_and_nothing_for_a_missing_one() {
    let me = procfs_state(std::process::id()).expect("own state");
    assert!(!me.cpus_allowed.is_empty() && !me.cgroup.is_empty());
    assert_eq!(procfs_state(u32::MAX), None);
}

mod attribution {
    use stutter_core::ids::{ExperimentId, Pid};

    use super::*;

    fn held(id: &str, action: ResearchAction, pid: u32) -> Holding {
        Holding {
            experiment_id: ExperimentId::new(id),
            action: Some(action),
            pid: Some(Pid::new(pid)),
        }
    }

    fn with(holdings: Vec<Holding>, active: bool, kept: usize, tokens: Option<u32>) -> GateView {
        GateView {
            holdings,
            ..view(active, kept, tokens)
        }
    }

    #[test]
    fn the_permitted_actions_own_experiment_is_the_transaction_in_the_receipt() {
        let mut g = gate().with_state_probe(Box::new(|_| {
            Some(KernelState {
                cpus_allowed: "0-15".into(),
                cgroup: "0::/a".into(),
                ..Default::default()
            })
        }));
        assert!(g.permit(Some(CPU), target(), NOW));
        g.observe(with(vec![held("exp-1", CPU, 4242)], true, 0, Some(1)));
        g.observe(with(vec![held("exp-1", CPU, 4242)], true, 0, Some(1)));
        g.observe(with(vec![], false, 0, Some(0)));
        let evidence = g.evidence();
        assert!(evidence.clean && evidence.kernel_state_proven);
        assert_eq!(
            evidence.receipts[0].transaction,
            Some(ExperimentId::new("exp-1"))
        );
    }

    #[test]
    fn another_action_another_process_or_another_experiment_is_not_attributable() {
        for foreign in [
            held("exp-1", CGROUP, 4242),
            held("exp-1", CPU, 999),
            held("exp-2", CPU, 4242),
        ] {
            let mut g = gate();
            assert!(g.permit(Some(CPU), target(), NOW));
            g.observe(with(vec![held("exp-1", CPU, 4242)], true, 0, Some(1)));
            // Something else is held beside it: the evidence cannot be about the permitted action.
            g.observe(with(
                vec![held("exp-1", CPU, 4242), foreign],
                true,
                0,
                Some(2),
            ));
            g.observe(with(vec![], false, 0, Some(0)));
            let evidence = g.evidence();
            assert!(!evidence.clean, "a foreign holding is never clean");
            assert!(!evidence.kernel_state_proven);
        }
    }

    #[test]
    fn re_arming_for_a_new_child_starts_from_clean_receipts() {
        let mut g = gate().with_state_probe(Box::new(|_| {
            Some(KernelState {
                cpus_allowed: "0-15".into(),
                cgroup: "0::/a".into(),
                ..Default::default()
            })
        }));
        assert!(g.permit(Some(CPU), target(), NOW));
        g.observe(with(vec![held("exp-1", CPU, 4242)], true, 0, Some(1)));
        g.observe(with(vec![], false, 0, Some(0)));
        assert!(g.disarm().clean);
        assert!(g.arm(ActionChildScope::new([CPU], target(), NOW + 60)));
        assert!(g.evidence().receipts.is_empty() && g.evidence().clean);
    }
}

#[test]
fn an_attempt_treated_off_never_permits_an_action_and_arming_resets_it() {
    let mut g = gate();
    g.set_attempt_on(false);
    assert!(
        !g.permit(Some(CPU), target(), NOW),
        "an Off attempt applies nothing"
    );
    assert!(!g.is_busy());
    g.set_attempt_on(true);
    assert!(g.permit(Some(CPU), target(), NOW));
    // A new child's arming starts from permitting again.
    let mut again = gate();
    again.set_attempt_on(false);
    assert!(again.arm(ActionChildScope::new([CPU], target(), NOW + 60)));
    assert!(again.permit(Some(CPU), target(), NOW));
}

mod fail_closed {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn state(identity: Option<(u64, u32)>, tid: u32, cpus: &str) -> KernelState {
        KernelState {
            cpus_allowed: "0-15".into(),
            cgroup: "0::/a".into(),
            tasks: [(4242, "0-15".to_owned()), (tid, cpus.to_owned())].into(),
            identity,
        }
    }

    fn probed(shared: &Arc<Mutex<Option<KernelState>>>) -> ActionGate {
        let shared = shared.clone();
        gate().with_state_probe(Box::new(move |_| shared.lock().unwrap().clone()))
    }

    #[test]
    fn an_experiment_reported_with_nothing_identifying_it_never_progresses() {
        let mut g = gate();
        assert!(g.permit(Some(CPU), target(), NOW));
        let mut unattributed = view(true, 0, Some(1));
        unattributed.holdings.clear();
        g.observe(unattributed);
        g.observe(view(false, 0, Some(0)));
        let evidence = g.evidence();
        assert!(!evidence.clean && !evidence.kernel_state_proven);
        assert!(evidence.settled.is_empty());
    }

    #[test]
    fn a_reused_pid_a_moved_thread_or_a_dead_probe_is_not_a_restoration() {
        let original = state(Some((99, 1000)), 4243, "0-15");
        for (after, what) in [
            (state(Some((100, 1000)), 4243, "0-15"), "the pid was reused"),
            (
                state(Some((99, 1000)), 4243, "2-3"),
                "a worker thread is still pinned",
            ),
            (state(Some((99, 0)), 4243, "0-15"), "another owner"),
        ] {
            let shared = Arc::new(Mutex::new(Some(original.clone())));
            let mut g = probed(&shared);
            assert!(g.permit(Some(CPU), target(), NOW));
            g.observe(view(true, 0, Some(1)));
            *shared.lock().unwrap() = Some(after);
            g.observe(view(false, 0, Some(0)));
            assert!(!g.evidence().clean, "{what}");
            assert!(g.evidence().receipts.is_empty(), "{what}");
        }
    }

    #[test]
    fn a_thread_that_exited_is_not_a_difference_but_a_moved_one_is() {
        let original = state(Some((99, 1000)), 4243, "0-15");
        let mut gone = original.clone();
        gone.tasks.remove(&4243);
        assert!(
            gone.restored_to(&original),
            "an exited thread is not a difference"
        );
        let moved = state(Some((99, 1000)), 4243, "1");
        assert!(!moved.restored_to(&original));
        // A thread the original never saw does not matter either.
        let mut spawned = original.clone();
        spawned.tasks.insert(9999, "1".into());
        assert!(spawned.restored_to(&original));
    }

    #[test]
    fn an_interrupted_restoration_stays_restoring_until_the_kernel_agrees() {
        let original = state(Some((99, 1000)), 4243, "0-15");
        let shared = Arc::new(Mutex::new(Some(original.clone())));
        let mut g = probed(&shared);
        assert!(g.permit(Some(CPU), target(), NOW));
        g.observe(view(true, 0, Some(1)));
        *shared.lock().unwrap() = Some(state(Some((99, 1000)), 4243, "1"));
        g.observe(view(false, 0, Some(0)));
        g.observe(view(false, 0, Some(0)));
        assert!(g.is_busy() && !g.evidence().clean);
        *shared.lock().unwrap() = Some(original);
        g.observe(view(false, 0, Some(0)));
        assert!(g.evidence().clean && g.evidence().kernel_state_proven);
    }
}

#[test]
fn an_unreadable_live_thread_is_not_an_exited_one() {
    use std::io::{Error, ErrorKind};
    let ok = |cpus: &str| Ok(format!("Name:\tgame\nCpus_allowed_list:\t{cpus}\n"));
    let tasks = parse_tasks([(1u32, ok("0-15")), (2, ok("2-3"))].into_iter()).unwrap();
    assert_eq!(tasks.len(), 2);
    // A thread that exited while being read is simply gone.
    let exited = parse_tasks(
        [
            (1u32, ok("0-15")),
            (2, Err(Error::from(ErrorKind::NotFound))),
        ]
        .into_iter(),
    )
    .unwrap();
    assert_eq!(exited.keys().copied().collect::<Vec<_>>(), [1]);
    // A thread that is still there but cannot be read makes the whole answer unknown.
    assert_eq!(
        parse_tasks(
            [
                (1u32, ok("0-15")),
                (2, Err(Error::from(ErrorKind::PermissionDenied)))
            ]
            .into_iter()
        ),
        None
    );
    // A status with no CPU list at all is not a restoration either.
    assert_eq!(
        parse_tasks([(1u32, Ok("Name:\tx\n".to_owned()))].into_iter()),
        None
    );
}

mod rebinding {
    use super::*;

    fn next_target() -> ChildTarget {
        ChildTarget {
            pid: stutter_core::ids::Pid::new(5555),
            start_ticks: 123,
            uid: 1000,
        }
    }

    #[test]
    fn each_attempt_is_its_own_process_and_its_own_scope_and_outcomes_accumulate() {
        let mut g = gate();
        // First attempt: apply, keep nothing, restore.
        assert!(g.permit(Some(CPU), target(), NOW));
        g.observe(view(true, 0, Some(1)));
        g.observe(view(false, 0, Some(0)));
        assert_eq!(g.evidence().settled, [(CPU, Decision::Revert)]);
        // The same action cannot be tried again on the same process...
        assert!(!g.permit(Some(CPU), target(), NOW));
        // ...nor on the next launch's process until the gate is moved onto it.
        assert!(!g.permit(Some(CPU), next_target(), NOW));
        assert!(g.rebind(next_target()));
        assert!(
            !g.permit(Some(CPU), target(), NOW),
            "the old process is no longer the target"
        );
        assert!(g.permit(Some(CPU), next_target(), NOW));
    }

    #[test]
    fn the_gate_does_not_move_while_an_action_is_outstanding_or_before_it_is_armed() {
        let mut g = gate();
        assert!(g.permit(Some(CPU), target(), NOW));
        assert!(!g.rebind(next_target()), "busy with an outstanding action");
        let mut unarmed = ActionGate::unarmed(Box::new(|| None));
        assert!(!unarmed.rebind(next_target()));
    }

    #[test]
    fn what_earlier_attempts_settled_stays_in_the_evidence() {
        let state = |cpus: &str| KernelState {
            cpus_allowed: cpus.into(),
            cgroup: "0::/a".into(),
            ..Default::default()
        };
        let mut g = gate().with_state_probe(Box::new(move |_| Some(state("0-15"))));
        for (index, process) in [target(), next_target()].into_iter().enumerate() {
            if index > 0 {
                assert!(g.rebind(process));
            }
            assert!(g.permit(Some(CPU), process, NOW));
            // The controller's manager holds the action on this attempt's own process.
            let mut held = view(true, 0, Some(1));
            held.holdings[0].pid = Some(process.pid);
            g.observe(held);
            g.observe(view(false, 0, Some(0)));
        }
        let evidence = g.evidence();
        assert!(evidence.clean && evidence.kernel_state_proven);
        assert_eq!(evidence.settled.len(), 2);
        assert_eq!(evidence.receipts.len(), 2);
    }
}

#[test]
fn a_truncated_or_failed_thread_listing_leaves_the_state_unknown() {
    use std::{ffi::OsString, path::PathBuf};
    let item = |tid: u32| {
        Ok((
            OsString::from(tid.to_string()),
            PathBuf::from(format!("/t/{tid}")),
        ))
    };
    // A complete listing, with an entry that vanished while listing skipped.
    let vanished = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
    let listed = task_entries([item(1), vanished, item(2)].into_iter(), 4).unwrap();
    assert_eq!(
        listed.iter().map(|(tid, _)| *tid).collect::<Vec<_>>(),
        vec![1, 2]
    );
    // More threads than the bound: unknown, never the first few passed off as the whole.
    assert_eq!(task_entries((0..5).map(item), 4), None);
    assert_eq!(task_entries((0..4).map(item), 4).map(|l| l.len()), Some(4));
    // A listing that fails part-way for any other reason (the old `.flatten()` dropped these).
    let denied = Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    assert_eq!(
        task_entries([item(1), denied, item(2)].into_iter(), 4),
        None
    );
    // A name that is not a thread id is not a thread.
    let odd = Ok((OsString::from("zero"), PathBuf::from("/t/zero")));
    assert_eq!(task_entries([item(1), odd].into_iter(), 4), None);
}

#[test]
fn a_process_that_changed_during_the_capture_has_no_identity() {
    let target = |pid: u32, ticks: u64| ChildTarget {
        pid: stutter_core::ids::Pid::new(pid),
        start_ticks: ticks,
        uid: 1000,
    };
    assert_eq!(
        steady_identity(Some(target(7, 50)), Some(target(7, 50))),
        Some((50, 1000))
    );
    // The pid was reused part-way (another start time), or either observation failed.
    assert_eq!(
        steady_identity(Some(target(7, 50)), Some(target(7, 51))),
        None
    );
    assert_eq!(steady_identity(Some(target(7, 50)), None), None);
    assert_eq!(steady_identity(None, Some(target(7, 50))), None);
    // The live state of this very process is capturable, with an identity.
    let own = procfs_state(std::process::id()).expect("own state");
    assert!(own.identity.is_some() && !own.tasks.is_empty());
}
