use std::collections::BTreeSet;

use crate::validation_campaign::{
    graph::{Graph, SourceBinding, TaskKind, TaskSpec, core_validation_tasks, validation_gates},
    model::CampaignPhase,
    runners::default_runners,
};

fn spec(id: &'static str, depends_on: &'static [&'static str]) -> TaskSpec {
    TaskSpec {
        id,
        kind: TaskKind::OfflineReady,
        phase: CampaignPhase::Preflight,
        depends_on,
        binding: SourceBinding::None,
        optional: false,
    }
}

#[test]
fn the_core_graph_is_valid_and_orders_dependencies_first() {
    let graph = Graph::core().unwrap();
    let order = graph.topological();
    let position = |id: &str| order.iter().position(|x| *x == id).unwrap();
    for spec in graph.specs() {
        for dependency in spec.depends_on {
            assert!(
                position(dependency) < position(spec.id),
                "{} before {dependency}",
                spec.id
            );
        }
    }
    assert_eq!(order.first(), Some(&"offline-ready"));
    assert_eq!(order.last(), Some(&"tag"));
}

#[test]
fn release_depends_on_everything_that_must_precede_it() {
    let graph = Graph::core().unwrap();
    let dependents = graph.dependents("offline-ready");
    for id in [
        "evidence-integration",
        "capability-policy",
        "freeze-rc",
        "release-verify",
        "tag",
    ] {
        assert!(dependents.contains(id), "{id}");
    }
    let tag = graph.get("tag").unwrap();
    assert!(
        tag.depends_on.contains(&"release-verify") && tag.depends_on.contains(&"install-smoke")
    );
    let verify = graph.get("release-verify").unwrap();
    for rc in [
        "rc-live-observation",
        "rc-game-runnable",
        "rc-affinity",
        "rc-recovery",
    ] {
        assert!(verify.depends_on.contains(&rc), "{rc}");
    }
    // Calibration and score review precede the capability policy, which precedes the freeze.
    let policy = graph.get("capability-policy").unwrap();
    assert!(
        policy.depends_on.contains(&"diagnosis-calibration")
            && policy.depends_on.contains(&"score-review")
    );
}

#[test]
fn validation_gates_name_real_tasks_and_cover_every_area() {
    let graph = Graph::core().unwrap();
    let gates = validation_gates();
    assert_eq!(
        gates
            .iter()
            .map(|g| g.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "observation",
            "kernel-compatibility",
            "profiler-overhead",
            "diagnosis-corpus",
            "diagnosis-calibration",
            "score-evaluation",
            "capability-policy",
            "release-readiness",
        ])
    );
    for gate in &gates {
        for task in &gate.tasks {
            assert!(
                graph.get(task).is_some(),
                "{} names unknown task {task}",
                gate.name
            );
        }
    }
    let by_name = |name: &str| gates.iter().find(|g| g.name == name).unwrap();
    assert_eq!(
        by_name("observation").tasks,
        ["live-observation", "game-runnable"]
    );
    assert!(
        by_name("profiler-overhead")
            .tasks
            .contains(&"overhead-real".to_owned())
    );
    let release = by_name("release-readiness");
    for task in [
        "tag",
        "release-verify",
        "install-smoke",
        "rc-offline",
        "rc-recovery",
    ] {
        assert!(release.tasks.iter().any(|t| t == task), "{task}");
    }
}

#[test]
fn every_task_kind_has_a_registered_runner() {
    let runners = default_runners();
    for spec in core_validation_tasks() {
        assert!(runners.contains_key(&spec.kind), "{:?}", spec.kind);
    }
}

#[test]
fn bad_graphs_are_rejected() {
    assert!(
        Graph::new(vec![spec("a", &[]), spec("a", &[])]).is_err(),
        "duplicate"
    );
    assert!(
        Graph::new(vec![spec("a", &["missing"])]).is_err(),
        "unknown dependency"
    );
    let cycle = Graph::new(vec![spec("a", &["b"]), spec("b", &["a"])]);
    assert!(cycle.is_err(), "cycle");
    assert!(Graph::new(vec![spec("a", &[]), spec("b", &["a"])]).is_ok());
}

#[test]
fn only_optional_tasks_can_be_skipped_by_policy() {
    let graph = Graph::core().unwrap();
    assert!(graph.check_skip_policy(&[]).is_ok());
    assert!(
        graph
            .check_skip_policy(&["live-observation".to_owned()])
            .is_err()
    );
    assert!(
        graph
            .check_skip_policy(&["no-such-task".to_owned()])
            .is_err()
    );
    let mut optional = spec("opt", &[]);
    optional.optional = true;
    let custom = Graph::new(vec![optional]).unwrap();
    assert!(custom.check_skip_policy(&["opt".to_owned()]).is_ok());
}
