use std::{cell::RefCell, fs};

use super::{
    Scratch,
    capture_exec::{FakeHost, Script},
    fresh, manifest,
};
use crate::validation_campaign::{
    corpus::{CorpusRunner, HostFactory, SCENARIO_FILE, corpus_dir, plan, rejection_counts},
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::Graph,
    model::{BlockReason, FailureClass},
    repository::Binaries,
    scenarios::{exec::CaptureHost, model::ScenarioSet},
};

struct Factory {
    fail_at: Option<&'static str>,
    missing_tool: bool,
}

impl HostFactory for Factory {
    fn host(
        &self,
        _: &RunContext<'_>,
        _: &ScenarioSet,
    ) -> Result<Box<dyn CaptureHost>, BlockReason> {
        if self.missing_tool {
            return Err(BlockReason {
                class: FailureClass::BlockedResource,
                detail: "vkmark is not on PATH".to_owned(),
            });
        }
        Ok(Box::new(FakeHost::new(Script {
            fail_at: self.fail_at,
            frames: 60,
            sanitize_writes_frames: true,
            ..Script::default()
        })))
    }
    fn source_matches(&self, _: &RunContext<'_>) -> bool {
        true
    }
}

fn run(factory: Factory, scratch: &Scratch) -> (RunnerOutcome, ScenarioSet) {
    let (dir, state, _) = fresh(scratch);
    let set = super::scenarios_set();
    let scenario_path = scratch.path().join(SCENARIO_FILE);
    fs::create_dir_all(scenario_path.parent().unwrap()).unwrap();
    fs::write(
        &scenario_path,
        include_str!("../../../../docs/validation/scenarios/core-validation.toml"),
    )
    .unwrap();
    let manifest = manifest();
    let binaries = Binaries {
        stutter: "stutter".into(),
        xtask: "xtask".into(),
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
    let spec = Graph::core().unwrap().get("corpus").unwrap().clone();
    let outcome = CorpusRunner {
        factory: Box::new(factory),
    }
    .run(&context, &spec, 1);
    (outcome, set)
}

#[test]
fn the_corpus_task_generates_every_floor_and_records_the_scenario_digest() {
    let scratch = Scratch::new("corpus-pass");
    let (outcome, set) = run(
        Factory {
            fail_at: None,
            missing_tool: false,
        },
        &scratch,
    );
    let RunnerOutcome::Passed { fingerprint, .. } = outcome else {
        panic!("the corpus did not complete");
    };
    assert!(fingerprint.scenario_sha256.is_some());
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
    let (progress, _) = plan(&set, &corpus_dir(&context), &context);
    assert!(progress.complete());
    assert!(corpus_dir(&context).join("summary.json").is_file());
    assert!(!corpus_dir(&context).join("rejections.ndjson").exists());
}

#[test]
fn persistent_capture_failure_ends_in_an_honest_resource_block_with_the_rejections_retained() {
    let scratch = Scratch::new("corpus-exhaust");
    let (outcome, _) = run(
        Factory {
            fail_at: Some("record"),
            missing_tool: false,
        },
        &scratch,
    );
    let RunnerOutcome::Blocked(block) = outcome else {
        panic!("a corpus that never captures must block");
    };
    assert_eq!(block.class, FailureClass::BlockedResource);
    assert!(block.detail.contains("exhausted"), "{}", block.detail);
    let (dir, ..) = fresh(&scratch);
    let counts = rejection_counts(&dir.0.join("corpus"));
    assert!(counts.values().all(|n| *n <= 3 * 10), "{counts:?}");
    assert!(counts.values().sum::<usize>() >= 3);
}

#[test]
fn a_missing_tool_blocks_before_any_capture_starts() {
    let scratch = Scratch::new("corpus-tool");
    let (outcome, _) = run(
        Factory {
            fail_at: None,
            missing_tool: true,
        },
        &scratch,
    );
    assert!(matches!(outcome, RunnerOutcome::Blocked(b) if b.detail.contains("vkmark")));
}
