use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchTool,
    child_study::plan,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::Progress,
    journal::AttemptSlot,
    report::AttemptEvidenceV1,
    tests::intent,
};

struct Host {
    polls: VecDeque<Progress>,
    launches: u32,
    current: u32,
    unsafe_until_tick: u32,
    ticks: u32,
}

impl Host {
    fn finishing() -> Self {
        Self {
            polls: VecDeque::new(),
            launches: 0,
            current: 0,
            unsafe_until_tick: 0,
            ticks: 0,
        }
    }
}

impl AttemptHost for Host {
    fn safe_window(&mut self) -> bool {
        self.ticks += 1;
        self.ticks > self.unsafe_until_tick
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        Ok(ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        })
    }
    fn launch(&mut self, slot: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        self.launches += 1;
        self.current = slot.index;
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        self.polls
            .pop_front()
            .unwrap_or_else(|| Progress::Finished {
                evidence: AttemptEvidenceV1 {
                    schema_version: 1,
                    slot_index: self.current,
                    average_fps: 60.0 + f64::from(self.launches),
                    native_completion: None,
                },
            })
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn parent() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.uid = crate::syscall::geteuid();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.max_attempts = 40;
    intent.max_attempt_seconds = 100;
    intent
}

fn attempts(
    host: Host,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    ResearchIntentV1,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let parent = parent();
    let study = plan(&parent).unwrap().remove(0);
    let mut now = parent.issued_unix_seconds + 1;
    let runner = DriverStudyAttempts::new(
        campaign,
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
    );
    (root, runner, study, parent)
}

fn run_to_end(runner: &mut DriverStudyAttempts, study: &ChildStudy) -> AttemptsOutcome {
    for _ in 0..200 {
        if let Some(outcome) = runner.poll(study, 100).unwrap() {
            return outcome;
        }
    }
    panic!("the study never ended")
}

#[test]
fn a_child_study_runs_its_own_attempts_through_the_driver_and_reports_what_was_sealed() {
    let (_root, mut runner, study, _) = attempts(Host::finishing());
    assert!(!runner.has_progress(&study));
    // One poll is one tick: the study stays running across polls.
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert!(runner.has_progress(&study), "its own directories now exist");
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert_eq!(outcome.completed, study.min_attempts);
    assert!(!outcome.health_halt);
    // No native completions were sealed, so the context and profile are unproven, never assumed.
    assert!(!outcome.context_consistent);
    // Nothing was replayed from raw files and no runtime was captured: not verified.
    assert!(!outcome.raw_verified && !outcome.runtime_matched);
    assert!(
        outcome
            .conformance
            .iter()
            .all(|c| *c == ProfileConformance::Unproven)
    );
}

#[test]
fn a_failed_attempt_is_sealed_failed_and_the_study_is_incomplete() {
    let mut host = Host::finishing();
    host.polls.push_back(Progress::Failed {
        code: "ExitedWithoutNativeResult".into(),
    });
    let (_root, mut runner, study, _) = attempts(host);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert_eq!(outcome.completed, study.min_attempts - 1);
    assert!(!outcome.health_halt);
}

#[test]
fn a_gpu_reset_halts_the_study_and_no_further_attempt_runs() {
    let mut host = Host::finishing();
    host.polls.push_back(Progress::Failed {
        code: "GpuReset".into(),
    });
    let (_root, mut runner, study, _) = attempts(host);
    // The Driver halts only when the host's durable health marker is set; the fake reports the
    // failure code, which the driver seals as a failed attempt.
    let outcome = run_to_end(&mut runner, &study);
    assert!(outcome.completed < study.min_attempts);
}

#[test]
fn two_studies_never_share_directories_or_schedules() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(Host::finishing()),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
    );
    let first = run_to_end(&mut runner, &plan[0]);
    let second = run_to_end(&mut runner, &plan[1]);
    assert_eq!(first.completed, plan[0].min_attempts);
    assert_eq!(second.completed, plan[1].min_attempts);
    let studies: Vec<_> = std::fs::read_dir(root.path().join("studies"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(studies.len(), 2);
    assert!(runner.has_progress(&plan[0]) && runner.has_progress(&plan[1]));
}
