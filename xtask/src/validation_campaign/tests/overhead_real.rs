use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    backend::{BackendError, SuiteBackend, SuiteInvocation},
    executor::{RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::Graph,
    model::FailureClass,
    overhead_real::{FROZEN_CYCLES, FROZEN_WORKLOAD, OverheadRealRunner},
    repository::Binaries,
};

struct Denied;

impl SuiteBackend for Denied {
    fn execute(&self, invocation: &SuiteInvocation<'_>) -> Result<i32, BackendError> {
        // The real-workload suite always carries its frozen cycle count.
        assert!(invocation.work_units.is_some());
        Err(BackendError::Privilege("not authorized".to_owned()))
    }
}

fn run(
    frozen: Option<&str>,
    timer_calls: &Rc<Cell<u32>>,
) -> (RunnerOutcome, Vec<(String, String)>) {
    let scratch = Scratch::new("overhead-real");
    let (dir, mut state, _) = fresh(&scratch);
    if let Some(value) = frozen {
        state
            .frozen
            .insert(FROZEN_CYCLES.to_owned(), value.to_owned());
    }
    let manifest = manifest();
    let binaries = Binaries {
        stutter: "s".into(),
        xtask: "x".into(),
    };
    let updates = RefCell::new(Vec::new());
    let context = RunContext {
        dir: &dir,
        state: &state,
        manifest: &manifest,
        repository: scratch.path(),
        binaries: &binaries,
        updates: &updates,
    };
    let calls = Rc::clone(timer_calls);
    let runner = OverheadRealRunner {
        time_one_cycle: Box::new(move |_| {
            calls.set(calls.get() + 1);
            Ok(10.0)
        }),
        backend: || Box::new(Denied),
    };
    let spec = Graph::core().unwrap().get("overhead-real").unwrap().clone();
    let outcome = runner.run(&context, &spec, 1);
    let frozen = updates
        .borrow()
        .iter()
        .filter_map(|u| match u {
            StateUpdate::Freeze { key, value } => Some((key.clone(), value.clone())),
            _ => None,
        })
        .collect();
    (outcome, frozen)
}

#[test]
fn the_workload_is_calibrated_once_frozen_and_bound_to_the_fingerprint() {
    let calls = Rc::new(Cell::new(0));
    let (outcome, frozen) = run(None, &calls);
    assert_eq!(calls.get(), 1);
    // A 10 s cycle against the manifest window needs a handful of cycles, never zero.
    let cycles: u64 = frozen
        .iter()
        .find(|(k, _)| k == FROZEN_CYCLES)
        .unwrap()
        .1
        .parse()
        .unwrap();
    assert!(cycles >= 1);
    assert!(
        frozen
            .iter()
            .any(|(k, v)| k == FROZEN_WORKLOAD && v.len() == 64)
    );
    // Without privilege this is an honest block, not a failure or a pass.
    assert!(
        matches!(outcome, RunnerOutcome::Blocked(b) if b.class == FailureClass::BlockedPrivilege)
    );
}

#[test]
fn a_frozen_cycle_count_is_never_recalibrated() {
    let calls = Rc::new(Cell::new(0));
    let (_, frozen) = run(Some("7"), &calls);
    assert_eq!(calls.get(), 0);
    assert!(!frozen.iter().any(|(k, _)| k == FROZEN_CYCLES));
}
