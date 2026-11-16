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
    GateView {
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
    assert!(g.permit(Some(CGROUP), target(), NOW));
    g.observe(view(true, 0, Some(1)));
    g.observe(view(false, 1, Some(1))); // kept
    assert!(!g.evidence().clean);
    assert!(
        !g.permit(Some(CPU), target(), NOW),
        "a kept action is still a live mutation"
    );
    g.observe(view(false, 1, Some(1)));
    g.observe(view(false, 0, Some(0))); // released and restored
    assert_eq!(g.evidence().settled, [(CGROUP, Decision::Keep)]);
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
    g.observe_runtime(true, 0);
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
