use std::{collections::BTreeMap, fs, path::Path};

use super::{MANIFEST_TEXT, Scratch, manifest};
use crate::{
    diagnosis_calibration::{CalibrationRole, FAMILIES, family_floors, family_spec, parse_fixture},
    validation_campaign::scenarios::{
        CorpusPlan,
        capture::{
            CaptureEvidence, InterventionReport, NextAction, accept, failure_limit, next_action,
        },
        family_summary,
        fixture::{
            CaptureRecord, HostFacts, count_artifacts, fixture_name, render, sanitized_capture_id,
        },
        model::{ControlType, Family, InterventionKind, ScenarioSet},
        short_families,
    },
};

const SCENARIOS: &str = include_str!("../../../../docs/validation/scenarios/core-validation.toml");

fn set() -> ScenarioSet {
    ScenarioSet::parse(SCENARIOS).unwrap()
}

fn edited(from: &str, to: &str) -> String {
    assert!(SCENARIOS.contains(from), "{from}");
    SCENARIOS.replacen(from, to, 1)
}

#[test]
fn the_committed_scenarios_deliver_every_floor_with_66_independent_captures() {
    let set = set();
    set.validate_against(&manifest()).unwrap();
    assert_eq!(set.total_captures(), 66);
    for family in Family::ALL {
        let (positive, negative, mixed) = set.capacity(family);
        assert_eq!(
            (positive, negative, mixed),
            (10, 10, 3),
            "{}",
            family.as_str()
        );
    }
    assert!(MANIFEST_TEXT.contains("min_positive = 10"));
}

#[test]
fn inconsistent_scenario_sets_are_rejected() {
    for (from, to) in [
        ("id = \"io-control\"", "id = \"clean-vkmark\""),
        (
            "expected_primary_cause = \"GameThreadSchedulerDelay\"\nseconds = 30\ncount = 10",
            "expected_primary_cause = \"BlockIoCandidate\"\nseconds = 30\ncount = 10",
        ),
        (
            "interventions = []\nexpected_primary_cause = \"Unknown\"\nseconds = 30\ncount = 10\n\n[[scenario]]\nid = \"io-control\"",
            "interventions = [\"scheduler_burst\"]\nexpected_primary_cause = \"Unknown\"\nseconds = 30\ncount = 10\n\n[[scenario]]\nid = \"io-control\"",
        ),
        (
            "interventions = [\"scheduler_burst\", \"blocking_io\"]",
            "interventions = [\"scheduler_burst\"]",
        ),
        (
            "covers = [\"scheduler\"]\ntarget = \"vkmark_light\"\ninterventions = [\"scheduler_burst\"]",
            "covers = [\"scheduler\", \"io\"]\ntarget = \"vkmark_light\"\ninterventions = [\"scheduler_burst\"]",
        ),
        ("light_size = \"640x480\"", "light_size = \"big\""),
        (
            "seconds = 30\ncount = 10\n\n[[scenario]]\nid = \"io-control\"",
            "seconds = 5\ncount = 10\n\n[[scenario]]\nid = \"io-control\"",
        ),
        (
            "present_mode = \"fifo\"",
            "present_mode = \"fifo\"\nsurprise = 1",
        ),
    ] {
        assert!(
            ScenarioSet::parse(&edited(from, to)).is_err(),
            "{from} -> {to}"
        );
    }
}

#[test]
fn a_set_that_cannot_meet_a_floor_is_rejected_against_the_manifest() {
    let short = ScenarioSet::parse(&edited("id = \"scheduler-positive\"\ncontrol = \"positive\"\ncovers = [\"scheduler\"]\ntarget = \"vkmark_light\"\ninterventions = [\"scheduler_burst\"]\nexpected_primary_cause = \"GameThreadSchedulerDelay\"\nseconds = 30\ncount = 10", "id = \"scheduler-positive\"\ncontrol = \"positive\"\ncovers = [\"scheduler\"]\ntarget = \"vkmark_light\"\ninterventions = [\"scheduler_burst\"]\nexpected_primary_cause = \"GameThreadSchedulerDelay\"\nseconds = 30\ncount = 9")).unwrap();
    let error = short.validate_against(&manifest()).unwrap_err().to_string();
    assert!(error.contains("scheduler"), "{error}");
}

fn host() -> HostFacts {
    HostFacts {
        kernel: "6.18.55".into(),
        distribution: "ID=testos".into(),
        cpu_vendor: "GenuineTest".into(),
        cpu_topology_bucket: "8c16t".into(),
        gpu_vendor: "AMD".into(),
        gpu_driver: "amdgpu".into(),
        compositor: "Hyprland".into(),
        session_type: "wayland".into(),
        workload_version: "vkmark-2025.01".into(),
    }
}

fn counts() -> BTreeMap<String, u64> {
    BTreeMap::from([("intervals".to_owned(), 30), ("frames".to_owned(), 1800)])
}

fn rendered(set: &ScenarioSet, scenario: &str, attempt: u32, raw: &str, quality: &str) -> String {
    let spec = set.get(scenario).unwrap();
    let host = host();
    let counts = counts();
    render(&CaptureRecord {
        spec,
        campaign_id: "core-abc-1",
        attempt,
        raw_capture_id: raw,
        source_commit: "0123456789abcdef0123456789abcdef01234567",
        host: &host,
        probe_set: "core",
        drop_counters: 0,
        data_quality: quality,
        artifact_counts: &counts,
    })
    .unwrap()
}

#[test]
fn generated_fixtures_use_the_one_corpus_contract_and_the_intervention_as_ground_truth() {
    let set = set();
    let text = rendered(&set, "scheduler-positive", 1, "raw-1", "High");
    let value: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(value["evidence_kind"].as_str(), Some("fixture_real"));
    assert_eq!(
        value["expected"]["primary_cause"].as_str(),
        Some("GameThreadSchedulerDelay")
    );
    assert_eq!(
        value["provenance"]["control_type"].as_str(),
        Some("positive")
    );
    assert_eq!(value["coverage"]["diagnoses"].as_array().unwrap().len(), 1);
    assert_eq!(value["privacy"]["usernames_redacted"].as_bool(), Some(true));
    for key in crate::fixture_coverage::REQUIRED_PROVENANCE {
        assert!(value["provenance"].get(key).is_some(), "{key}");
    }
    // Ground truth comes only from the scenario: the data-quality statement cannot change it.
    let low = rendered(&set, "scheduler-positive", 1, "raw-1", "Low");
    let low: toml::Value = toml::from_str(&low).unwrap();
    assert_eq!(
        low["expected"]["primary_cause"],
        value["expected"]["primary_cause"]
    );
    assert_eq!(
        low["provenance"]["control_type"],
        value["provenance"]["control_type"]
    );
}

#[test]
fn generated_fixtures_are_calibration_eligible_with_the_right_role_per_family() {
    let set = set();
    let role = |scenario: &str, family: &str| {
        let fixture = parse_fixture(Path::new("d"), &rendered(&set, scenario, 1, "raw", "High"));
        assert!(
            fixture.meta.is_some(),
            "{scenario}: {}",
            fixture.ineligible_reason
        );
        fixture.role(family_spec(family).unwrap())
    };
    assert_eq!(
        role("scheduler-positive", "scheduler"),
        CalibrationRole::Positive
    );
    assert_eq!(
        role("scheduler-positive", "gpu"),
        CalibrationRole::Ineligible
    );
    assert_eq!(
        role("clean-vkmark", "scheduler"),
        CalibrationRole::NegativeControl
    );
    assert_eq!(
        role("clean-vkmark", "irq"),
        CalibrationRole::NegativeControl
    );
    assert_eq!(
        role("clean-vkmark", "io"),
        CalibrationRole::Ineligible,
        "vkmark is not a matched I/O control"
    );
    assert_eq!(role("io-control", "io"), CalibrationRole::NegativeControl);
    assert_eq!(
        role("mixed-scheduler-io", "scheduler"),
        CalibrationRole::MixedConfounded
    );
    assert_eq!(
        role("mixed-scheduler-io", "io"),
        CalibrationRole::MixedConfounded
    );
    assert_eq!(
        role("mixed-scheduler-io", "irq"),
        CalibrationRole::Ineligible
    );
    assert_eq!(role("gpu-positive", "gpu"), CalibrationRole::Positive);
}

#[test]
fn generated_fixtures_pass_fixture_coverage_provenance_and_privacy_checks() {
    let set = set();
    let root = Scratch::new("coverage-root");
    let runs = root.path().join("stutter/tests/fixtures/runs");
    for (n, scenario) in ["scheduler-positive", "clean-vkmark", "mixed-irq-gpu"]
        .into_iter()
        .enumerate()
    {
        let text = rendered(&set, scenario, n as u32 + 1, &format!("raw-{n}"), "High");
        let name = toml::from_str::<toml::Value>(&text).unwrap()["name"]
            .as_str()
            .unwrap()
            .to_owned();
        fs::create_dir_all(runs.join(&name)).unwrap();
        fs::write(runs.join(&name).join("fixture.toml"), text).unwrap();
    }
    let report = crate::fixture_coverage::fixture_coverage_report(root.path()).unwrap();
    assert!(
        report.provenance_errors.is_empty(),
        "{:?}",
        report.provenance_errors
    );
    assert!(
        report.privacy_warnings.is_empty(),
        "{:?}",
        report.privacy_warnings
    );
    assert!(
        report.maturity.tag_errors.is_empty(),
        "{:?}",
        report.maturity.tag_errors
    );
    assert_eq!(report.real_fixture_count, 3);
}

#[test]
fn capture_ids_are_stable_private_and_attempt_specific() {
    let id = sanitized_capture_id("core-1", "scheduler-positive", 1, "raw");
    assert_eq!(
        id,
        sanitized_capture_id("core-1", "scheduler-positive", 1, "raw")
    );
    assert_ne!(
        id,
        sanitized_capture_id("core-1", "scheduler-positive", 2, "raw")
    );
    assert_ne!(
        id,
        sanitized_capture_id("core-2", "scheduler-positive", 1, "raw")
    );
    assert!(id.starts_with("sc-") && id.len() == 27 && !id.contains("raw"));
    let name = fixture_name("scheduler-positive", &id);
    assert!(
        name.starts_with("real_scheduler_positive_") && !name.contains('-'),
        "{name}"
    );
}

#[test]
fn artifact_counts_are_exact_line_counts() {
    let scratch = Scratch::new("artifact-counts");
    fs::write(scratch.path().join("interval.json"), "{}\n{}\n\n{}\n").unwrap();
    fs::write(scratch.path().join("frame_correlation.json"), "{}\n").unwrap();
    let counts = count_artifacts(scratch.path());
    assert_eq!(
        (counts["intervals"], counts["frames"], counts["irq_events"]),
        (3, 1, 0)
    );
}

fn good(spec: &crate::validation_campaign::scenarios::model::ScenarioSpec) -> CaptureEvidence {
    CaptureEvidence {
        target_identity_valid: true,
        recording_valid: true,
        drops: 0,
        write_errors: 0,
        interventions: spec
            .interventions
            .iter()
            .map(|kind| InterventionReport {
                kind: *kind,
                verified: true,
                detail: String::new(),
            })
            .collect(),
        fingerprint_matches: true,
        capture_id_unique: true,
        sanitizer_ok: true,
        fixture_valid: true,
        frame_samples: 1800,
    }
}

#[test]
fn a_capture_counts_only_when_every_acceptance_condition_holds() {
    let set = set();
    let spec = set.get("scheduler-positive").unwrap();
    accept(spec, &good(spec)).unwrap();
    type Mutation = Box<dyn Fn(&mut CaptureEvidence)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        ("identity", Box::new(|e| e.target_identity_valid = false)),
        ("recording", Box::new(|e| e.recording_valid = false)),
        ("dropped", Box::new(|e| e.drops = 1)),
        ("write errors", Box::new(|e| e.write_errors = 1)),
        ("fingerprint", Box::new(|e| e.fingerprint_matches = false)),
        ("unique", Box::new(|e| e.capture_id_unique = false)),
        ("sanitizer", Box::new(|e| e.sanitizer_ok = false)),
        ("fixture", Box::new(|e| e.fixture_valid = false)),
        ("frames", Box::new(|e| e.frame_samples = 0)),
        (
            "intervention",
            Box::new(|e| e.interventions[0].verified = false),
        ),
        (
            "missing intervention",
            Box::new(|e| e.interventions.clear()),
        ),
    ];
    for (name, mutate) in mutations {
        let mut evidence = good(spec);
        mutate(&mut evidence);
        assert!(accept(spec, &evidence).is_err(), "{name}");
    }
    // A control must not carry an active intervention.
    let control = set.get("clean-vkmark").unwrap();
    accept(control, &good(control)).unwrap();
    let mut dirty = good(control);
    dirty.interventions.push(InterventionReport {
        kind: InterventionKind::SchedulerBurst,
        verified: true,
        detail: String::new(),
    });
    let reasons = accept(control, &dirty).unwrap_err();
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("unexpected intervention")),
        "{reasons:?}"
    );
    // fio targets have no frame stream requirement.
    let fio = set.get("io-control").unwrap();
    let mut fio_evidence = good(fio);
    fio_evidence.frame_samples = 0;
    accept(fio, &fio_evidence).unwrap();
}

#[test]
fn the_retry_budget_is_per_required_capture_and_ends_in_exhaustion() {
    assert_eq!(failure_limit(3, 10), 30);
    assert_eq!(next_action(10, 10, 99, 3), NextAction::Complete);
    assert_eq!(next_action(4, 10, 29, 3), NextAction::Capture);
    assert_eq!(
        next_action(4, 10, 30, 3),
        NextAction::Exhausted {
            failed: 30,
            limit: 30
        }
    );
}

#[test]
fn the_corpus_plan_counts_eligible_captures_once_and_schedules_the_deficit() {
    let set = set();
    let mut fixtures = Vec::new();
    for n in 0..8 {
        fixtures.push(parse_fixture(
            Path::new("d"),
            &rendered(&set, "scheduler-positive", n + 1, &format!("r{n}"), "High"),
        ));
    }
    // A repeated capture id (same attempt and raw id) counts once; an unknown scenario never.
    fixtures.push(parse_fixture(
        Path::new("d"),
        &rendered(&set, "scheduler-positive", 1, "r0", "High"),
    ));
    let mut foreign = parse_fixture(
        Path::new("d"),
        &rendered(&set, "clean-vkmark", 1, "x", "High"),
    );
    foreign.scenario = "not-a-scenario".to_owned();
    fixtures.push(foreign);
    let plan = CorpusPlan::from_fixtures(&set, &fixtures, &BTreeMap::new(), 3);
    let scheduler = plan
        .scenarios
        .iter()
        .find(|s| s.id == "scheduler-positive")
        .unwrap();
    assert_eq!((scheduler.have, scheduler.need), (8, 10));
    assert!(!plan.complete());
    let next = plan.next_scenarios(100);
    assert_eq!(
        next.iter().filter(|id| *id == "scheduler-positive").count(),
        2
    );
    assert_eq!(next.len(), 66 - 8);
    // Exhausting one scenario's budget removes it from the schedule and reports it.
    let failed = BTreeMap::from([("scheduler-positive".to_owned(), 30)]);
    let plan = CorpusPlan::from_fixtures(&set, &fixtures, &failed, 3);
    assert!(
        !plan
            .next_scenarios(100)
            .contains(&"scheduler-positive".to_owned())
    );
    assert!(plan.exhausted().unwrap().contains("scheduler-positive"));
    // Family floors use the calibrator's own accounting.
    let min = (10, 10, 3);
    let summary = family_summary(&fixtures, min, &plan);
    assert_eq!(summary["scheduler"]["positive"]["have"], 8);
    assert_eq!(summary["scheduler"]["positive"]["need"], 10);
    assert_eq!(family_floors(&FAMILIES[0], &fixtures).positives, 8);
    assert_eq!(short_families(&fixtures, min).len(), 4);
    assert_eq!(ControlType::NegativeControl.as_str(), "negative_control");
}

#[test]
fn intervention_telemetry_is_judged_from_independent_proc_and_sysfs_facts() {
    use crate::validation_campaign::scenarios::probe::*;
    // Field 14/15 utime/stime = 300/100 ticks; rt_priority (40) = 50; policy (41) = 1.
    let filler_before = (4..14).map(|_| "0").collect::<Vec<_>>().join(" ");
    let filler_mid = (16..40).map(|_| "0").collect::<Vec<_>>().join(" ");
    let stat = |policy: u32, rt: u32, utime: u64| {
        format!("77 (live-burst) R {filler_before} {utime} 100 {filler_mid} {rt} {policy} 0")
    };
    assert_eq!(parse_sched(&stat(1, 50, 300)), Some((1, 50, 400)));
    // 4 ms of every 20 ms over 30 s is 600 ticks expected; half of that is the floor.
    scheduler_burst_established(&stat(1, 50, 300), 50, 4, 20, 30).unwrap();
    assert!(
        scheduler_burst_established(&stat(0, 50, 900), 50, 4, 20, 30)
            .unwrap_err()
            .contains("SCHED_FIFO")
    );
    assert!(scheduler_burst_established(&stat(1, 10, 900), 50, 4, 20, 30).is_err());
    assert!(
        scheduler_burst_established(&stat(1, 50, 50), 50, 4, 20, 30)
            .unwrap_err()
            .contains("below half")
    );
    assert!(scheduler_burst_established("garbage", 50, 4, 20, 30).is_err());
    gpu_busy_established(&[95, 99, 97, 96], 90, 4).unwrap();
    assert!(
        gpu_busy_established(&[95, 99], 90, 4)
            .unwrap_err()
            .contains("fewer")
    );
    assert!(
        gpu_busy_established(&[40, 50, 60, 70], 90, 4)
            .unwrap_err()
            .contains("below")
    );
}

#[test]
fn calibration_walks_a_bounded_ladder_and_stops_on_the_first_working_step() {
    use crate::validation_campaign::scenarios::probe::first_passing;
    let mut seen = Vec::new();
    let found = first_passing(&[1, 2, 3, 4], &mut |step| {
        seen.push(*step);
        Ok(*step >= 3)
    });
    assert_eq!((found, seen.as_slice()), (Ok(Some(2)), &[1, 2, 3][..]));
    assert_eq!(first_passing(&[1, 2], &mut |_| Ok(false)), Ok(None));
    assert!(first_passing(&[1, 2], &mut |_| Err("cannot probe".into())).is_err());
}

#[test]
fn host_facts_are_bucketed_and_carry_no_machine_identity() {
    use crate::validation_campaign::scenarios::probe::*;
    assert_eq!(
        (
            gpu_vendor("0x1002\n"),
            gpu_vendor("0x10DE"),
            gpu_vendor("0x8086"),
            gpu_vendor("0x1")
        ),
        ("AMD", "NVIDIA", "Intel", "Unknown")
    );
    assert_eq!(
        os_release_summary("NAME=\"Test OS\"\nID=testos\nVERSION_ID=\"12\"\nHOME_URL=x\n"),
        "ID=testos VERSION_ID=12"
    );
    let cpuinfo = "processor : 0\nvendor_id : GenuineTest\nmodel name : Secret CPU 9000\ncpu cores : 4\nprocessor : 1\nvendor_id : GenuineTest\n";
    let (vendor, bucket) = cpu_bucket(cpuinfo);
    assert_eq!((vendor.as_str(), bucket.as_str()), ("GenuineTest", "4c2t"));
    assert!(!format!("{vendor}{bucket}").contains("Secret"));
}

#[test]
fn proc_stat_fields_are_read_past_hostile_command_names() {
    use crate::validation_campaign::system_host::{last_cpu, starttime_ticks};
    // Fields 4..21 filler, field 22 = start time, fields 23..38 filler, field 39 = processor.
    let before = (4..22).map(|_| "0").collect::<Vec<_>>().join(" ");
    let between = (23..39).map(|_| "0").collect::<Vec<_>>().join(" ");
    let stat = format!("9 (a) b (c) S {before} 4242 {between} 6 0");
    assert_eq!(
        (starttime_ticks(&stat), last_cpu(&stat)),
        (Some(4242), Some(6))
    );
    assert_eq!(starttime_ticks("nonsense"), None);
}
