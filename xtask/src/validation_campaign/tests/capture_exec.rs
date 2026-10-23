use std::{collections::BTreeSet, fs, path::Path};

use super::{Scratch, scenarios_set};
use crate::{
    diagnosis_calibration::{CalibrationRole, family_spec, parse_fixture},
    validation_campaign::scenarios::{
        capture::InterventionReport,
        exec::{
            CaptureHost, CaptureRequest, CaptureResult, RecordSummary, TargetIdentity, run_capture,
        },
        fixture::HostFacts,
        model::ScenarioSpec,
    },
};

#[derive(Default)]
pub(super) struct Script {
    pub(super) fail_at: Option<&'static str>,
    pub(super) unverified: bool,
    pub(super) unexpected_burst: bool,
    pub(super) frames: u64,
    pub(super) sanitize_writes_frames: bool,
}

pub(super) struct FakeHost {
    script: Script,
    log: Vec<String>,
}

impl FakeHost {
    pub(super) fn new(script: Script) -> Self {
        Self {
            script,
            log: Vec::new(),
        }
    }
    fn step(&mut self, name: &'static str) -> Result<(), String> {
        self.log.push(name.to_owned());
        if self.script.fail_at == Some(name) {
            Err(format!("{name} failed"))
        } else {
            Ok(())
        }
    }
}

impl CaptureHost for FakeHost {
    fn preflight(&mut self, _: &ScenarioSpec) -> Result<(), String> {
        self.step("preflight")
    }
    fn start_target(&mut self, _: &ScenarioSpec) -> Result<TargetIdentity, String> {
        self.step("target")?;
        Ok(TargetIdentity {
            pid: 4242,
            starttime_ticks: 777,
        })
    }
    fn start_interventions(&mut self, _: &ScenarioSpec, _: TargetIdentity) -> Result<(), String> {
        self.step("interventions")
    }
    fn record(
        &mut self,
        _: &ScenarioSpec,
        _: TargetIdentity,
        raw: &Path,
    ) -> Result<RecordSummary, String> {
        self.step("record")?;
        fs::create_dir_all(raw).unwrap();
        fs::write(raw.join("interval.json"), "{}\n{}\n").unwrap();
        Ok(RecordSummary {
            valid: true,
            drops: 0,
            replaced_wakeups: 0,
            write_errors: 0,
            frame_samples: self.script.frames,
        })
    }
    fn verify_interventions(&mut self, spec: &ScenarioSpec, _: &Path) -> Vec<InterventionReport> {
        self.log.push("verify".to_owned());
        let mut reports = spec
            .interventions
            .iter()
            .map(|kind| InterventionReport {
                kind: *kind,
                verified: !self.script.unverified,
                detail: String::new(),
            })
            .collect::<Vec<_>>();
        if self.script.unexpected_burst {
            reports.push(InterventionReport {
                kind:
                    crate::validation_campaign::scenarios::model::InterventionKind::SchedulerBurst,
                verified: true,
                detail: String::new(),
            });
        }
        reports
    }
    fn stop_all(&mut self) {
        self.log.push("stop".to_owned());
    }
    fn sanitize(&mut self, raw: &Path, out: &Path, _: &str) -> Result<(), String> {
        self.step("sanitize")?;
        fs::create_dir_all(out).unwrap();
        fs::copy(raw.join("interval.json"), out.join("interval.json")).unwrap();
        if self.script.sanitize_writes_frames {
            fs::write(out.join("frame_correlation.json"), "{}\n".repeat(60)).unwrap();
        }
        Ok(())
    }
    fn data_quality(&mut self, _: &Path) -> Result<String, String> {
        self.step("quality").map(|()| "High".to_owned())
    }
    fn validate(&mut self, _: &Path) -> Result<(), String> {
        self.step("validate")
    }
    fn facts(&self) -> (HostFacts, String) {
        (
            HostFacts {
                kernel: "6.18.55".into(),
                distribution: "ID=t".into(),
                cpu_vendor: "V".into(),
                cpu_topology_bucket: "8c16t".into(),
                gpu_vendor: "G".into(),
                gpu_driver: "d".into(),
                compositor: "c".into(),
                session_type: "wayland".into(),
                workload_version: "v1".into(),
            },
            "corpus".to_owned(),
        )
    }
}

struct Run {
    _scratch: Scratch,
    fixtures: std::path::PathBuf,
    result: CaptureResult,
    log: Vec<String>,
}

fn capture(scenario: &str, script: Script, published: &[&str], fingerprint_ok: bool) -> Run {
    let scratch = Scratch::new("capture");
    let set = scenarios_set();
    let spec = set.get(scenario).unwrap();
    let fixtures = scratch.path().join("corpus/fixtures");
    fs::create_dir_all(&fixtures).unwrap();
    let raw = scratch.path().join("raw");
    let ids: BTreeSet<String> = published.iter().map(|s| (*s).to_owned()).collect();
    let mut host = FakeHost::new(script);
    let fingerprint = move || fingerprint_ok;
    let result = run_capture(
        &mut host,
        &CaptureRequest {
            spec,
            campaign_id: "core-1",
            attempt: 1,
            raw_capture_id: "raw-1",
            source_commit: "0123456789abcdef0123456789abcdef01234567",
            raw_dir: &raw,
            fixtures_dir: &fixtures,
            published_ids: &ids,
            fingerprint_matches: &fingerprint,
        },
    );
    Run {
        _scratch: scratch,
        fixtures,
        result,
        log: host.log,
    }
}

fn published(run: &Run) -> Vec<String> {
    fs::read_dir(&run.fixtures)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn an_accepted_capture_runs_the_fixed_sequence_and_publishes_one_eligible_fixture() {
    let run = capture(
        "scheduler-positive",
        Script {
            frames: 1800,
            ..Script::default()
        },
        &[],
        true,
    );
    let CaptureResult::Accepted {
        fixture_dir,
        capture_id,
    } = &run.result
    else {
        panic!("{:?}", run.result);
    };
    assert_eq!(
        run.log,
        [
            "preflight",
            "target",
            "interventions",
            "record",
            "verify",
            "stop",
            "sanitize",
            "quality",
            "validate"
        ],
        "interventions are verified while active, then everything stops before processing"
    );
    assert_eq!(
        published(&run),
        [fixture_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()],
        "no staging leftovers"
    );
    let text = fs::read_to_string(fixture_dir.join("fixture.toml")).unwrap();
    let fixture = parse_fixture(Path::new("d"), &text);
    assert!(fixture.meta.is_some(), "{}", fixture.ineligible_reason);
    assert_eq!(
        fixture.role(family_spec("scheduler").unwrap()),
        CalibrationRole::Positive
    );
    assert_eq!(fixture.expected_cause, "GameThreadSchedulerDelay");
    assert!(text.contains(capture_id.as_str()));
    assert_eq!(fixture.scenario, "scheduler-positive");
}

#[test]
fn every_failure_stops_whatever_was_started_and_publishes_nothing() {
    for (stage, expected_stop) in [("target", true), ("interventions", true), ("record", true)] {
        let run = capture(
            "scheduler-positive",
            Script {
                fail_at: Some(stage),
                frames: 10,
                ..Script::default()
            },
            &[],
            true,
        );
        let CaptureResult::Rejected { reasons, .. } = &run.result else {
            panic!("{stage}: {:?}", run.result)
        };
        assert!(reasons[0].contains(stage), "{reasons:?}");
        assert_eq!(
            run.log.iter().any(|l| l == "stop"),
            expected_stop,
            "{stage}: {:?}",
            run.log
        );
        assert!(published(&run).is_empty(), "{stage}");
    }
    let run = capture(
        "scheduler-positive",
        Script {
            fail_at: Some("preflight"),
            ..Script::default()
        },
        &[],
        true,
    );
    assert!(matches!(&run.result, CaptureResult::Rejected { stage, .. } if stage == "preflight"));
    assert_eq!(
        run.log,
        ["preflight"],
        "nothing starts when preflight fails"
    );
}

#[test]
fn an_intervention_that_did_not_establish_itself_does_not_count() {
    let run = capture(
        "scheduler-positive",
        Script {
            unverified: true,
            frames: 1800,
            ..Script::default()
        },
        &[],
        true,
    );
    let CaptureResult::Rejected { stage, reasons } = &run.result else {
        panic!("{:?}", run.result)
    };
    assert_eq!(stage, "acceptance");
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("not independently verified")),
        "{reasons:?}"
    );
    assert!(published(&run).is_empty());
}

#[test]
fn a_control_with_an_unexpected_active_intervention_is_rejected() {
    let run = capture(
        "clean-vkmark",
        Script {
            unexpected_burst: true,
            frames: 1800,
            ..Script::default()
        },
        &[],
        true,
    );
    let CaptureResult::Rejected { reasons, .. } = &run.result else {
        panic!("{:?}", run.result)
    };
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("unexpected intervention")),
        "{reasons:?}"
    );
}

#[test]
fn sanitizer_validation_duplicate_and_fingerprint_failures_each_block_publication() {
    for (name, script, ids, fingerprint, needle) in [
        (
            "sanitizer",
            Script {
                fail_at: Some("sanitize"),
                frames: 1800,
                ..Script::default()
            },
            vec![],
            true,
            "sanitizer",
        ),
        (
            "validator",
            Script {
                fail_at: Some("validate"),
                frames: 1800,
                ..Script::default()
            },
            vec![],
            true,
            "fixture does not validate",
        ),
        (
            "fingerprint",
            Script {
                frames: 1800,
                ..Script::default()
            },
            vec![],
            false,
            "fingerprint",
        ),
    ] {
        let run = capture("scheduler-positive", script, &ids, fingerprint);
        let CaptureResult::Rejected { reasons, .. } = &run.result else {
            panic!("{name}: {:?}", run.result)
        };
        assert!(
            reasons.iter().any(|r| r.contains(needle)),
            "{name}: {reasons:?}"
        );
        assert!(published(&run).is_empty(), "{name}: {:?}", published(&run));
    }
    // The capture id is deterministic from the request, so a pre-published id is a duplicate.
    let id = crate::validation_campaign::scenarios::fixture::sanitized_capture_id(
        "core-1",
        "scheduler-positive",
        1,
        "raw-1",
    );
    let run = capture(
        "scheduler-positive",
        Script {
            frames: 1800,
            ..Script::default()
        },
        &[id.as_str()],
        true,
    );
    let CaptureResult::Rejected { reasons, .. } = &run.result else {
        panic!("{:?}", run.result)
    };
    assert!(
        reasons.iter().any(|r| r.contains("not unique")),
        "{reasons:?}"
    );
}

#[test]
fn a_frame_target_needs_frames_from_the_recording_or_the_sanitized_stream() {
    let none = capture("clean-vkmark", Script::default(), &[], true);
    assert!(
        matches!(&none.result, CaptureResult::Rejected { reasons, .. } if reasons.iter().any(|r| r.contains("frame stream")))
    );
    // The sanitized recording carries the frames even if the summary under-reported them.
    let from_stream = capture(
        "clean-vkmark",
        Script {
            sanitize_writes_frames: true,
            ..Script::default()
        },
        &[],
        true,
    );
    assert!(
        matches!(from_stream.result, CaptureResult::Accepted { .. }),
        "{:?}",
        from_stream.result
    );
    // A fio target is not a frame source.
    let fio = capture("io-control", Script::default(), &[], true);
    assert!(
        matches!(fio.result, CaptureResult::Accepted { .. }),
        "{:?}",
        fio.result
    );
}
