//! The multi-kernel stage against a simulated machine: boot entries, one-shot selection,
//! reboots (the controller is "restarted" by reloading state from disk), duplicate kernels,
//! boot failures, unexpected entries, budget exhaustion and restoring the original entry.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
};

use serde_json::json;

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{AfterPersist, Executor, RunContext, RunnerOutcome, TaskRunner},
    graph::{Graph, TaskKind, TaskSpec, core_validation_tasks},
    kernels::{
        BootFacts, CompatObservation, HostError, KernelHost, KernelMatrixRunner, KernelView, Step,
        TASK, observed_kernels, plan,
    },
    model::{Boundary, CampaignState, FailureClass, TaskStatus},
    repository::Binaries,
    state::{CampaignDir, load, save, write_atomic},
};

#[derive(Default)]
struct Machine {
    backend: String,
    /// entry id -> kernel release it boots.
    entries: Vec<(String, String)>,
    allowed: Vec<String>,
    original: Option<String>,
    current: String,
    reboots_used: u32,
    max_reboots: u32,
    oneshot: Option<String>,
    /// Entries that fail to boot and fall back to the original (firmware fallback).
    unbootable: BTreeSet<String>,
    reboot_fails: bool,
    compat_runs: u32,
    resumes: u32,
    reboot_calls: Vec<String>,
}

impl Machine {
    fn release_of(&self, entry: &str) -> String {
        self.entries
            .iter()
            .find(|(id, _)| id == entry)
            .map(|(_, k)| k.clone())
            .unwrap_or_default()
    }
}

struct FakeHost(Rc<RefCell<Machine>>);

impl KernelHost for FakeHost {
    fn boot_facts(&self, _: &RunContext<'_>) -> Result<BootFacts, HostError> {
        let m = self.0.borrow();
        Ok(BootFacts {
            backend: m.backend.clone(),
            current_entry: Some(m.current.clone()),
            kernel_release: m.release_of(&m.current),
            allowed: m.allowed.clone(),
            original: m.original.clone(),
            entry_versions: BTreeMap::new(),
            reboots_used: m.reboots_used,
            max_reboots: m.max_reboots,
        })
    }

    fn run_compat(
        &self,
        ctx: &RunContext<'_>,
        _: &TaskSpec,
        _: u32,
    ) -> Result<CompatObservation, RunnerOutcome> {
        let mut m = self.0.borrow_mut();
        m.compat_runs += 1;
        let kernel = m.release_of(&m.current);
        // Publish exactly what the real pipeline does: a verified index entry.
        let entry = json!({"capture_id": format!("cap-{}", m.compat_runs), "suite": "live_kernel_compat",
            "status": "PASS", "kernel_release": kernel, "compatibility": "ObservedCompatible"});
        write_atomic(
            &ctx.dir
                .indexes()
                .join(TASK)
                .join(format!("cap-{}.json", m.compat_runs)),
            &serde_json::to_vec(&entry).unwrap(),
        )
        .unwrap();
        Ok(CompatObservation {
            kernel_release: kernel,
        })
    }

    fn set_next_boot(&self, _: &RunContext<'_>, entry: &str) -> Result<(), HostError> {
        let mut m = self.0.borrow_mut();
        if !m.allowed.iter().any(|a| a == entry) {
            return Err(HostError::Failed(format!("{entry} is not authorized")));
        }
        m.oneshot = Some(entry.to_owned());
        Ok(())
    }

    fn reboot_action(&self, _: &RunContext<'_>) -> AfterPersist {
        let machine = Rc::clone(&self.0);
        Box::new(move || {
            let mut m = machine.borrow_mut();
            if m.reboot_fails {
                return Err("reboot refused".to_owned());
            }
            if m.reboots_used >= m.max_reboots {
                return Err("budget".to_owned());
            }
            m.reboots_used += 1;
            let wanted = m
                .oneshot
                .take()
                .or_else(|| m.original.clone())
                .unwrap_or_default();
            let landed = if m.unbootable.contains(&wanted) {
                m.original.clone().unwrap_or_default()
            } else {
                wanted.clone()
            };
            m.reboot_calls.push(format!("{wanted}->{landed}"));
            m.current = landed;
            Ok(())
        })
    }

    fn acknowledge_resume(&self, _: &RunContext<'_>) -> Result<(), HostError> {
        self.0.borrow_mut().resumes += 1;
        Ok(())
    }
}

struct PassAll;

impl TaskRunner for PassAll {
    fn run(&self, _: &RunContext<'_>, _: &TaskSpec, _: u32) -> RunnerOutcome {
        RunnerOutcome::passed()
    }
}

fn machine(entries: &[(&str, &str)], original: &str, max_reboots: u32) -> Rc<RefCell<Machine>> {
    Rc::new(RefCell::new(Machine {
        backend: "fake-boot".to_owned(),
        entries: entries
            .iter()
            .map(|(i, k)| ((*i).to_owned(), (*k).to_owned()))
            .collect(),
        allowed: entries.iter().map(|(i, _)| (*i).to_owned()).collect(),
        original: Some(original.to_owned()),
        current: original.to_owned(),
        max_reboots,
        ..Machine::default()
    }))
}

fn executor(host: Rc<RefCell<Machine>>) -> Executor {
    let mut runners: BTreeMap<TaskKind, Box<dyn TaskRunner>> = BTreeMap::new();
    for spec in core_validation_tasks() {
        runners.insert(spec.kind, Box::new(PassAll));
    }
    runners.insert(
        TaskKind::KernelMatrix,
        Box::new(KernelMatrixRunner {
            host: Box::new(FakeHost(host)),
        }),
    );
    Executor {
        graph: Graph::core().unwrap(),
        manifest: manifest(),
        runners,
        repository: PathBuf::from("/nonexistent"),
        binaries: Binaries {
            stutter: PathBuf::from("s"),
            xtask: PathBuf::from("x"),
        },
        source_check: Box::new(|_, _| Ok(())),
    }
}

/// Runs `continue` repeatedly, reloading state from disk each time exactly as a resumed
/// controller process would, until the campaign leaves the reboot loop.
fn drive(
    executor: &Executor,
    dir: &CampaignDir,
    state: CampaignState,
) -> (CampaignState, Vec<Boundary>) {
    let mut boundaries = Vec::new();
    save(dir, &state).unwrap();
    for _ in 0..12 {
        let mut state = load(dir).unwrap();
        let report = executor.continue_campaign(dir, &mut state, None).unwrap();
        boundaries.push(report.boundary);
        if report.boundary != Boundary::WaitingForReboot {
            return (load(dir).unwrap(), boundaries);
        }
    }
    panic!("the reboot loop did not terminate: {boundaries:?}");
}

fn setup(name: &str) -> (Scratch, CampaignDir, CampaignState) {
    let scratch = Scratch::new(name);
    let (dir, state, _) = fresh(&scratch);
    (scratch, dir, state)
}

fn view(
    observed: &[&str],
    tried: &[&str],
    current: &str,
    release: &str,
    pending: Option<&str>,
) -> KernelView {
    KernelView {
        required: 3,
        observed: observed.iter().map(|s| (*s).to_owned()).collect(),
        boot: BootFacts {
            backend: "fake-boot".into(),
            current_entry: Some(current.into()),
            kernel_release: release.into(),
            allowed: ["a", "b", "c", "d"].map(String::from).to_vec(),
            original: Some("a".into()),
            entry_versions: BTreeMap::new(),
            reboots_used: 0,
            max_reboots: 6,
        },
        tried: tried.iter().map(|s| (*s).to_owned()).collect(),
        pending: pending.map(str::to_owned),
    }
}

#[test]
fn the_planner_observes_first_then_walks_unused_entries_then_restores_the_original() {
    assert_eq!(plan(&view(&[], &[], "a", "k1", None)), Step::RunCompat);
    assert_eq!(
        plan(&view(&["k1"], &["a"], "a", "k1", None)),
        Step::Reboot {
            into: "b".into(),
            restore: false
        }
    );
    assert_eq!(
        plan(&view(&["k1", "k2"], &["a", "b"], "b", "k2", None)),
        Step::Reboot {
            into: "c".into(),
            restore: false
        }
    );
    // Enough distinct kernels: go home, and only then finish.
    assert_eq!(
        plan(&view(
            &["k1", "k2", "k3"],
            &["a", "b", "c"],
            "c",
            "k3",
            None
        )),
        Step::Reboot {
            into: "a".into(),
            restore: true
        }
    );
    assert_eq!(
        plan(&view(
            &["k1", "k2", "k3"],
            &["a", "b", "c"],
            "a",
            "k1",
            None
        )),
        Step::Done { restored: true }
    );
    // A reboot in flight must have landed on the authorized entry.
    assert_eq!(
        plan(&view(&["k1"], &["a"], "b", "k2", Some("b"))),
        Step::Arrived
    );
}

#[test]
fn the_planner_stops_rather_than_guess() {
    let blocked = |step: Step| matches!(step, Step::Blocked(_));
    // Landed somewhere else than authorized.
    assert!(blocked(plan(&view(&["k1"], &["a"], "a", "k1", Some("b")))));
    // Every candidate tried, too few kernels.
    assert!(blocked(plan(&view(
        &["k1", "k2"],
        &["a", "b", "c", "d"],
        "d",
        "k2",
        None
    ))));
    // Unknown boot backend.
    let mut unknown = view(&[], &[], "a", "k1", None);
    unknown.boot.backend = "unsupported".into();
    assert!(blocked(plan(&unknown)));
    // Budget exhausted before the next reboot, and before restoring.
    let mut spent = view(&["k1"], &["a"], "a", "k1", None);
    spent.boot.reboots_used = 6;
    assert!(blocked(plan(&spent)));
    let mut cannot_return = view(&["k1", "k2", "k3"], &["a", "b", "c"], "c", "k3", None);
    cannot_return.boot.reboots_used = 6;
    assert!(blocked(plan(&cannot_return)));
    // An entry whose reported version equals the running kernel cannot add a release.
    let mut same = view(&["k1"], &["a"], "a", "k1", None);
    same.boot.entry_versions.insert("b".into(), "k1".into());
    assert_eq!(
        plan(&same),
        Step::Reboot {
            into: "c".into(),
            restore: false
        }
    );
}

#[test]
fn three_distinct_kernels_are_observed_across_reboots_and_the_original_is_restored() {
    let (_scratch, dir, state) = setup("kernels-ok");
    let host = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 5);
    let (state, boundaries) = drive(&executor(host.clone()), &dir, state);
    assert_eq!(
        *boundaries.last().unwrap(),
        Boundary::Complete,
        "{boundaries:?}"
    );
    assert!(
        boundaries
            .iter()
            .filter(|b| **b == Boundary::WaitingForReboot)
            .count()
            >= 3
    );
    let m = host.borrow();
    assert_eq!(
        m.reboot_calls,
        ["b->b", "c->c", "a->a"],
        "visited b and c, then returned home"
    );
    assert_eq!(
        (m.current.as_str(), m.compat_runs, m.reboots_used),
        ("a", 3, 3)
    );
    assert!(m.resumes >= 3, "every reboot is acknowledged to the runner");
    let observed = observed_kernels(&dir);
    assert_eq!(
        observed.keys().cloned().collect::<Vec<_>>(),
        ["k1", "k2", "k3"]
    );
    assert_eq!(state.tasks[TASK].status, TaskStatus::Passed);
    assert_eq!(state.frozen["kernel.observed"], "k1,k2,k3");
    assert_eq!(state.frozen["kernel.pending"], "");
}

#[test]
fn a_duplicate_kernel_release_does_not_count_and_the_next_entry_is_tried() {
    let (_scratch, dir, state) = setup("kernels-dup");
    // b boots the same kernel as a: it adds nothing, so c and d are needed.
    let host = machine(
        &[("a", "k1"), ("b", "k1"), ("c", "k2"), ("d", "k3")],
        "a",
        6,
    );
    let (_, boundaries) = drive(&executor(host.clone()), &dir, state);
    assert_eq!(
        *boundaries.last().unwrap(),
        Boundary::Complete,
        "{boundaries:?}"
    );
    assert_eq!(observed_kernels(&dir).len(), 3);
    assert_eq!(host.borrow().current, "a");
    assert!(
        host.borrow()
            .reboot_calls
            .iter()
            .any(|c| c.starts_with("b->")),
        "the duplicate was tried"
    );
}

#[test]
fn a_boot_that_lands_on_the_wrong_entry_stops_autonomous_rebooting() {
    let (_scratch, dir, state) = setup("kernels-fail");
    let host = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 6);
    host.borrow_mut().unbootable.insert("b".to_owned());
    let (state, boundaries) = drive(&executor(host.clone()), &dir, state);
    assert_eq!(
        *boundaries.last().unwrap(),
        Boundary::BlockedResource,
        "{boundaries:?}"
    );
    let detail = &state.tasks[TASK].block.as_ref().unwrap().detail;
    assert!(detail.contains("authorized transition"), "{detail}");
    assert_eq!(
        host.borrow().reboots_used,
        1,
        "no further reboot after the unexpected entry"
    );
}

#[test]
fn the_reboot_budget_and_the_candidate_list_are_hard_limits() {
    let (_scratch, dir, state) = setup("kernels-budget");
    let host = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 1);
    let (state, boundaries) = drive(&executor(host.clone()), &dir, state);
    assert_eq!(
        *boundaries.last().unwrap(),
        Boundary::BlockedResource,
        "{boundaries:?}"
    );
    assert!(
        state.tasks[TASK]
            .block
            .as_ref()
            .unwrap()
            .detail
            .contains("budget")
    );
    assert_eq!(host.borrow().reboots_used, 1);

    let (_scratch2, dir2, state2) = setup("kernels-two");
    let two = machine(&[("a", "k1"), ("b", "k2")], "a", 6);
    let (state2, boundaries2) = drive(&executor(two.clone()), &dir2, state2);
    assert_eq!(
        *boundaries2.last().unwrap(),
        Boundary::BlockedResource,
        "{boundaries2:?}"
    );
    let detail = &state2.tasks[TASK].block.as_ref().unwrap().detail;
    assert!(
        detail.contains("all candidate boot entries were tried"),
        "{detail}"
    );
    assert_eq!(
        two.borrow().compat_runs,
        2,
        "the real observations are kept"
    );
}

#[test]
fn an_unknown_bootloader_and_a_refused_reboot_block_without_acting() {
    let (_scratch, dir, state) = setup("kernels-unknown");
    let host = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 5);
    host.borrow_mut().backend = "unsupported".to_owned();
    let (_, boundaries) = drive(&executor(host.clone()), &dir, state);
    assert_eq!(*boundaries.last().unwrap(), Boundary::BlockedResource);
    assert_eq!(
        host.borrow().compat_runs,
        0,
        "nothing runs on an unsupported host"
    );
    assert_eq!(host.borrow().reboots_used, 0);

    let (_scratch2, dir2, state2) = setup("kernels-refused");
    let refusing = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 5);
    refusing.borrow_mut().reboot_fails = true;
    let (state2, boundaries2) = drive(&executor(refusing.clone()), &dir2, state2);
    assert_eq!(
        *boundaries2.last().unwrap(),
        Boundary::BlockedResource,
        "{boundaries2:?}"
    );
    let attempts = &state2.tasks[TASK].attempts;
    assert!(
        matches!(&attempts.last().unwrap().outcome, crate::validation_campaign::model::AttemptOutcome::Blocked(r) if r.detail.contains("reboot request failed")),
        "{attempts:?}"
    );
    assert_eq!(refusing.borrow().reboots_used, 0);
    let _ = FailureClass::BlockedResource;
}

#[test]
fn a_task_cut_off_mid_run_is_recovered_and_the_waiting_state_survives_a_restart() {
    let (_scratch, dir, mut state) = setup("kernels-restart");
    // The controller died while a collection task was Running.
    state.tasks.get_mut("corpus").unwrap().status = TaskStatus::Running;
    let host = machine(&[("a", "k1"), ("b", "k2"), ("c", "k3")], "a", 5);
    let (state, _) = drive(&executor(host), &dir, state);
    let corpus = &state.tasks["corpus"];
    assert_eq!(corpus.status, TaskStatus::Passed);
    assert!(corpus.attempts.iter().any(|a| matches!(
        &a.outcome,
        crate::validation_campaign::model::AttemptOutcome::Failed { stage, .. } if stage == "interrupted"
    )), "the cut-off attempt is retained as history");
}
