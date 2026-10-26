use std::{cell::RefCell, fs, path::Path, time::Duration};

use serde_json::json;

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::Graph,
    landing_task::LandingTools,
    model::{FailureClass, TaskStatus},
    policy::{
        CapabilityPolicyRunner, REGISTRY, apply_policy, calibration_doc, policy_section,
        replace_marked, score_doc, unmet_gates,
    },
    repository::Binaries,
    shell::{Shell, ShellOutput},
};

const SOURCE: &str = include_str!("../../../../stutter/src/capability_tiers.rs");

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn policy() -> (Vec<String>, Vec<String>) {
    let m = manifest();
    (
        m.policy.capabilities.supported,
        m.policy.capabilities.experimental,
    )
}

/// `(tier, target_tier)` of one capability read back out of registry text.
fn tiers_of(source: &str, id: &str) -> (String, String) {
    let at = source.find(&format!("\"{id}\"")).unwrap();
    let span = &source[at..];
    let mut found = span.match_indices("CapabilityTier::").map(|(i, _)| {
        span[i + 16..]
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap()
            .to_owned()
    });
    (found.next().unwrap(), found.next().unwrap())
}

#[test]
fn the_policy_promotes_the_core_capabilities_and_the_graduated_cpu_affinity_and_cgroup_placement_and_keeps_the_rest_conservative()
 {
    let (supported, experimental) = policy();
    let out = apply_policy(SOURCE, &supported, &experimental).unwrap();
    for id in [
        "doctor",
        "record",
        "bench",
        "report",
        "audit",
        "completions_and_man",
        "cpu_affinity_experiment",
        "cgroup_placement_experiment",
    ] {
        assert_eq!(
            tiers_of(&out, id),
            ("Supported".into(), "Supported".into()),
            "{id}"
        );
    }
    for id in [
        "diagnose",
        "advisor",
        "tune_evaluation",
        "restore",
        "uclamp_experiment",
        "nice_experiment",
        "ioprio_experiment",
        "cpu_power_epp",
        "cpu_power_governor",
        "gpu_power_profile",
        "gpu_force_performance",
        "vm_swappiness",
        "vm_dirty_background_ratio",
        "vm_dirty_ratio",
        "irq_affinity_known_device",
        "irq_affinity_storage_or_unknown",
        "always_on_autotune",
        "remote_agent_mutation",
    ] {
        assert_eq!(
            tiers_of(&out, id),
            ("Experimental".into(), "Experimental".into()),
            "{id}"
        );
    }
    for id in ["privileged_worker", "fake_action_provider"] {
        assert_eq!(
            tiers_of(&out, id),
            ("Internal".into(), "Internal".into()),
            "{id}"
        );
    }
    // Idempotent, and untouched capabilities keep their text byte for byte.
    assert_eq!(apply_policy(&out, &supported, &experimental).unwrap(), out);
}

#[test]
fn an_unknown_capability_is_refused_before_anything_is_written() {
    let error = apply_policy(SOURCE, &ids(&["no_such_capability"]), &[]).unwrap_err();
    assert!(error.contains("no_such_capability"));
}

#[test]
fn the_documentation_block_is_replaced_in_place_never_duplicated() {
    let section = policy_section(&ids(&["a"]), &ids(&["b"]), &ids(&["c"]));
    let once = replace_marked("# Doc\n\ntext\n", &section);
    let twice = replace_marked(&once, &section);
    assert_eq!(once, twice);
    assert_eq!(twice.matches("v0.1.0 tier policy").count(), 1);
    let changed = replace_marked(&once, &policy_section(&ids(&["z"]), &[], &[]));
    assert!(changed.contains("`z`") && !changed.contains("`a`"));
}

#[test]
fn result_documents_carry_decisions_and_every_limitation() {
    let calibration = calibration_doc(
        &json!({"policy":"retain","families":{"io":{"decision":"retain_current_threshold"}},"limitations":["io is weak"],"diagnosis_capabilities":"Experimental"}),
        "core-1",
    );
    assert!(
        calibration.contains("| io | retain_current_threshold |")
            && calibration.contains("- io is weak")
    );
    let score = score_doc(
        &json!({"weights":"retained","runnable_monotonic":true,"frame_monotonic":false,"limitations":["frame series not monotonic"]}),
        "core-1",
    );
    assert!(
        score.contains("frame series monotonic: false")
            && score.contains("- frame series not monotonic")
    );
}

struct Landing;
impl Shell for Landing {
    fn run(&self, program: &str, args: &[&str], _: &Path) -> ShellOutput {
        let line = format!("{program} {}", args.join(" "));
        // No task branch exists yet, and the applied change leaves something staged.
        let code = i32::from(
            line.starts_with("git rev-parse --verify --quiet")
                || line.starts_with("git diff --cached --quiet"),
        );
        let out = if line.starts_with("git status --porcelain") {
            ""
        } else {
            "cafe\n"
        };
        ShellOutput {
            code,
            stdout: out.to_owned(),
            stderr: String::new(),
        }
    }
    fn sleep(&self, _: Duration) {}
}

fn run_policy(satisfy: bool) -> (RunnerOutcome, Scratch) {
    let scratch = Scratch::new("policy-run");
    let (dir, mut state, graph) = fresh(&scratch);
    if satisfy {
        for spec in graph.specs() {
            if spec.id != "capability-policy" {
                let fingerprint = state.expected_fingerprint(spec);
                let task = state.tasks.get_mut(spec.id).unwrap();
                task.status = TaskStatus::Passed;
                task.fingerprint = Some(fingerprint);
            }
        }
    }
    let repo = scratch.path();
    fs::create_dir_all(repo.join("stutter/src")).unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::write(repo.join(REGISTRY), SOURCE).unwrap();
    fs::write(repo.join("docs/CAPABILITIES.md"), "# Capability tiers\n").unwrap();
    fs::write(repo.join("docs/CORE_VALIDATION_STATUS.md"), "# Status\n").unwrap();
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
        repository: repo,
        binaries: &binaries,
        updates: &updates,
    };
    let runner = CapabilityPolicyRunner {
        tools: LandingTools {
            shell: Box::new(Landing),
            gates: Vec::new(),
        },
    };
    let spec = Graph::core()
        .unwrap()
        .get("capability-policy")
        .unwrap()
        .clone();
    let outcome = runner.run(&context, &spec, 1);
    (outcome, scratch)
}

#[test]
fn tiers_do_not_change_until_every_required_validation_gate_is_satisfied() {
    let (outcome, scratch) = run_policy(false);
    let RunnerOutcome::Blocked(block) = outcome else {
        panic!("must block")
    };
    assert_eq!(block.class, FailureClass::BlockedResource);
    assert_eq!(
        fs::read_to_string(scratch.path().join(REGISTRY)).unwrap(),
        SOURCE
    );
    let (dir, state, _) = fresh(&scratch);
    let manifest = manifest();
    let updates = RefCell::new(Vec::new());
    let binaries = Binaries {
        stutter: "s".into(),
        xtask: "x".into(),
    };
    let context = RunContext {
        dir: &dir,
        state: &state,
        manifest: &manifest,
        repository: scratch.path(),
        binaries: &binaries,
        updates: &updates,
    };
    assert_eq!(
        unmet_gates(&context),
        [
            "observation",
            "kernel-compatibility",
            "profiler-overhead",
            "diagnosis-corpus",
            "diagnosis-calibration",
            "score-evaluation",
        ]
    );
}

#[test]
fn a_satisfied_campaign_applies_the_registry_docs_and_ledger_in_one_local_landing() {
    let (outcome, scratch) = run_policy(true);
    assert!(
        matches!(outcome, RunnerOutcome::Passed { .. }),
        "{:?}",
        matches!(outcome, RunnerOutcome::Blocked(_))
    );
    let registry = fs::read_to_string(scratch.path().join(REGISTRY)).unwrap();
    assert_eq!(tiers_of(&registry, "doctor").0, "Supported");
    assert!(
        fs::read_to_string(scratch.path().join("docs/CAPABILITIES.md"))
            .unwrap()
            .contains("v0.1.0 tier policy")
    );
    assert!(
        fs::read_to_string(scratch.path().join("docs/CORE_VALIDATION_STATUS.md"))
            .unwrap()
            .contains("capability policy")
    );
}
