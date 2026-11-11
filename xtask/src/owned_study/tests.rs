use std::{collections::BTreeSet, path::PathBuf};

use super::{
    analyze::{analyze, bootstrap, verdict},
    manifest::{Arm, Manifest, Placement, effective_cpus_of_status, parse_cpulist},
    run::{Readback, Record, parse_readback, placed_command, readback_matches},
};

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owned-study-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

pub(super) fn allowed() -> BTreeSet<u32> {
    (0..12).collect()
}

pub(super) fn identity(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("identity.json");
    let text = serde_json::json!({"kind":"compile","source_tree":format!("git:{}", "a".repeat(40)),
        "target":"t","build_state":"cold","cache_policy":"none","job_count":2,
        "command":["cargo","build"],"working_directory":"/tmp/x","toolchain":"t","environment":"e"});
    std::fs::write(&path, text.to_string()).expect("identity");
    path
}

pub(super) fn arm(name: &str, compile: Placement, hogs: Placement) -> Arm {
    Arm {
        name: name.into(),
        compile,
        hogs,
    }
}

pub(super) fn manifest(dir: &std::path::Path) -> Manifest {
    Manifest {
        schema_version: 1,
        study_id: "demo".into(),
        identity: identity(dir),
        hog_threads: 12,
        blocks: 6,
        training_blocks: 3,
        arm_timeout_secs: 120,
        output_artifact: None,
        diagnostics: false,
        practical_percent: 2.0,
        baseline: arm("baseline", Placement::Unrestricted, Placement::Unrestricted),
        candidates: vec![
            arm(
                "compile-0-5",
                Placement::Cpus("0-5".into()),
                Placement::Unrestricted,
            ),
            arm(
                "hogs-6-11",
                Placement::Unrestricted,
                Placement::Cpus("6-11".into()),
            ),
            arm(
                "split",
                Placement::Cpus("0-5".into()),
                Placement::Cpus("6-11".into()),
            ),
            arm(
                "starved",
                Placement::Cpus("0-1".into()),
                Placement::Unrestricted,
            ),
        ],
    }
}

#[test]
fn a_well_formed_manifest_validates_and_digests_stably() {
    let dir = scratch("ok");
    let m = manifest(&dir);
    m.validate(&allowed()).expect("valid");
    assert_eq!(m.digest(), m.clone().digest());
}

type Mutation = Box<dyn Fn(&mut Manifest)>;

#[test]
fn unsafe_or_meaningless_manifests_are_refused_before_anything_runs() {
    let dir = scratch("bad");
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "cpu outside the effective set",
            Box::new(|m| m.candidates[0].compile = Placement::Cpus("0-12".into())),
        ),
        (
            "malformed cpulist",
            Box::new(|m| m.candidates[0].compile = Placement::Cpus("a-b".into())),
        ),
        (
            "no-op candidate",
            Box::new(|m| {
                m.candidates[0] = arm("noop", Placement::Unrestricted, Placement::Unrestricted)
            }),
        ),
        (
            "mixed family arm",
            Box::new(|m| m.candidates[0].compile = Placement::ScopeWeight(100)),
        ),
        (
            "duplicate arm",
            Box::new(|m| m.candidates[1].name = m.candidates[0].name.clone()),
        ),
        (
            "zero weight",
            Box::new(|m| {
                m.baseline = arm(
                    "baseline",
                    Placement::ScopeWeight(100),
                    Placement::ScopeWeight(100),
                );
                m.candidates = vec![arm(
                    "w",
                    Placement::ScopeWeight(0),
                    Placement::ScopeWeight(100),
                )];
            }),
        ),
        (
            "huge weight",
            Box::new(|m| {
                m.baseline = arm(
                    "baseline",
                    Placement::ScopeWeight(100),
                    Placement::ScopeWeight(100),
                );
                m.candidates = vec![arm(
                    "w",
                    Placement::ScopeWeight(10_001),
                    Placement::ScopeWeight(100),
                )];
            }),
        ),
        (
            "non neutral baseline",
            Box::new(|m| m.baseline.compile = Placement::Cpus("0-5".into())),
        ),
        (
            "baseline renamed",
            Box::new(|m| m.baseline.name = "control".into()),
        ),
        (
            "path-like name",
            Box::new(|m| m.candidates[0].name = "../etc".into()),
        ),
        ("too many blocks", Box::new(|m| m.blocks = 13)),
        ("no holdout", Box::new(|m| m.training_blocks = m.blocks)),
        ("timeout out of range", Box::new(|m| m.arm_timeout_secs = 5)),
        ("more hogs than cpus", Box::new(|m| m.hog_threads = 13)),
        ("zero threshold", Box::new(|m| m.practical_percent = 0.0)),
        (
            "too many candidates",
            Box::new(|m| {
                for i in 0..3 {
                    m.candidates.push(arm(
                        &format!("x{i}"),
                        Placement::Cpus("0-3".into()),
                        Placement::Unrestricted,
                    ));
                }
            }),
        ),
    ];
    for (what, mutate) in cases {
        let mut m = manifest(&dir);
        mutate(&mut m);
        assert!(m.validate(&allowed()).is_err(), "{what} must be refused");
    }
}

#[test]
fn unknown_fields_and_placement_kinds_do_not_deserialize() {
    let dir = scratch("serde");
    let mut value = serde_json::to_value(manifest(&dir)).expect("value");
    value["uid"] = 0.into();
    assert!(
        serde_json::from_value::<Manifest>(value).is_err(),
        "a UID field must not exist"
    );
    let bad = serde_json::json!({"name":"x","compile":{"kind":"Root","value":"/"},"hogs":{"kind":"Unrestricted"}});
    assert!(serde_json::from_value::<Arm>(bad).is_err());
}

#[test]
fn the_schedule_counterbalances_arms_and_never_repeats_a_slot() {
    let dir = scratch("sched");
    let m = manifest(&dir);
    let slots = m.schedule();
    assert_eq!(slots.len(), 6 * 5);
    assert_eq!(
        slots
            .iter()
            .map(|s| s.id.clone())
            .collect::<BTreeSet<_>>()
            .len(),
        slots.len()
    );
    for block in 1..=6 {
        let arms: BTreeSet<usize> = slots
            .iter()
            .filter(|s| s.block == block)
            .map(|s| s.arm)
            .collect();
        assert_eq!(arms.len(), 5, "every arm once per block");
    }
    let first_position: BTreeSet<usize> = (1..=5)
        .map(|b| {
            slots
                .iter()
                .find(|s| s.block == b)
                .map(|s| s.arm)
                .unwrap_or(99)
        })
        .collect();
    assert!(
        first_position.len() >= 3,
        "the leading arm varies across blocks"
    );
    assert_eq!(slots, m.schedule(), "deterministic");
}

#[test]
fn cpulists_parse_strictly() {
    assert_eq!(
        parse_cpulist("0-2,5").expect("list"),
        BTreeSet::from([0, 1, 2, 5])
    );
    for bad in ["", "3-1", "x", "1-2-3", "-1", "2000"] {
        assert!(parse_cpulist(bad).is_err(), "{bad}");
    }
    assert_eq!(
        effective_cpus_of_status("Name:\tx\nCpus_allowed_list:\t0-3\n")
            .expect("status")
            .len(),
        4
    );
    assert!(effective_cpus_of_status("Name:\tx\n").is_err());
}

#[test]
fn readback_must_match_the_request() {
    let stderr = "noise\nREADBACK compile cpus=0-5 weight=100\n";
    let read = parse_readback(stderr, "compile").expect("readback");
    assert_eq!(
        read,
        Readback {
            cpus: (0..6).collect(),
            weight: Some(100)
        }
    );
    assert!(readback_matches(
        &Placement::Cpus("0-5".into()),
        &allowed(),
        &read
    ));
    assert!(!readback_matches(
        &Placement::Cpus("0-4".into()),
        &allowed(),
        &read
    ));
    assert!(!readback_matches(
        &Placement::Unrestricted,
        &allowed(),
        &read
    ));
    assert!(readback_matches(
        &Placement::ScopeWeight(100),
        &allowed(),
        &read
    ));
    assert!(!readback_matches(
        &Placement::ScopeWeight(50),
        &allowed(),
        &read
    ));
    assert!(
        parse_readback(stderr, "hogs").is_err(),
        "another role's line is not this role's"
    );
    let none = parse_readback("READBACK hogs cpus=0-11 weight=\n", "hogs").expect("no weight");
    assert_eq!(none.weight, None);
    assert!(!readback_matches(
        &Placement::ScopeWeight(100),
        &allowed(),
        &none
    ));
}

#[test]
fn placed_commands_use_only_the_owned_launchers() {
    let inner = vec!["stress-ng".to_owned(), "--cpu".into(), "2".into()];
    let program = |placement: &Placement| {
        let c = placed_command("hogs", placement, "u", &inner);
        (
            c.get_program().to_string_lossy().into_owned(),
            c.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
        )
    };
    let (p, args) = program(&Placement::Cpus("6-11".into()));
    assert_eq!(
        (p.as_str(), &args[..3]),
        (
            "taskset",
            &["-c".to_owned(), "6-11".into(), "sh".into()][..]
        )
    );
    let (p, args) = program(&Placement::ScopeWeight(7));
    assert_eq!(p, "systemd-run");
    assert!(
        args.contains(&"--user".to_owned())
            && args.contains(&"--scope".to_owned())
            && args.contains(&"CPUWeight=7".to_owned())
    );
    let (p, args) = program(&Placement::Unrestricted);
    assert_eq!(p, "sh");
    assert_eq!(&args[args.len() - 3..], &inner[..]);
}

pub(super) fn record(block: u32, arm: &str, ns: u64, digest: &str) -> Record {
    Record {
        study_id: "demo".into(),
        manifest_sha256: digest.into(),
        slot: format!("b{block:02}-{arm}"),
        block,
        arm: arm.into(),
        valid: true,
        reason: None,
        elapsed_ns: Some(ns),
        context: None,
        output_sha256: None,
    }
}

/// Records for every slot of the schedule, in schedule order; `time` is None for an invalid slot.
pub(super) fn schedule_records(
    m: &Manifest,
    context: Option<&str>,
    time: impl Fn(u32, &str) -> Option<u64>,
) -> Vec<Record> {
    let arms = m.arms();
    m.schedule()
        .iter()
        .map(|slot| {
            let name = &arms[slot.arm].name;
            let mut r = record(slot.block, name, 0, &m.digest());
            r.context = context.map(str::to_owned);
            match time(slot.block, name) {
                Some(ns) => r.elapsed_ns = Some(ns),
                None => {
                    r.valid = false;
                    r.elapsed_ns = None;
                    r.reason = Some("test invalid".into());
                }
            }
            r
        })
        .collect()
}

/// Writes a legacy `results.jsonl` study directory (read-only analysis, no resume).
pub(super) fn write_legacy(dir: &std::path::Path, records: &[Record]) -> PathBuf {
    let text: String = records
        .iter()
        .map(|r| serde_json::to_string(r).expect("json") + "\n")
        .collect();
    std::fs::write(dir.join("results.jsonl"), text).expect("results");
    dir.to_path_buf()
}

fn standard_time(block: u32, arm: &str) -> Option<u64> {
    let wobble = u64::from(block % 2) * 20_000_000;
    Some(match arm {
        "baseline" => 8_000_000_000,
        "compile-0-5" => 8_000_000_000 + wobble,
        "hogs-6-11" => 6_000_000_000 + wobble,
        "split" => 5_000_000_000 + wobble,
        _ => 12_000_000_000 + wobble,
    })
}

#[test]
fn analysis_classifies_effects_and_ranks_only_legal_candidates() {
    let dir = scratch("analysis");
    let m = manifest(&dir);
    let records = schedule_records(&m, None, standard_time);
    let report = analyze(&m, &write_legacy(&dir, &records)).expect("analysis");
    assert_eq!(report["effects"]["hogs-6-11"]["verdict"], "Beneficial");
    assert_eq!(report["effects"]["split"]["verdict"], "Beneficial");
    assert_eq!(report["effects"]["starved"]["verdict"], "Harmful");
    assert_eq!(report["effects"]["compile-0-5"]["verdict"], "Equivalent");
    assert_eq!(report["training_favourite"]["candidate"], "split");
    assert_eq!(report["ranking"]["top1_agrees"], true);
    assert_eq!(report["authorizes_mutation"], false);
    assert_eq!(report["outside_space_dropped"], 0);
    assert_eq!(report["record_store"], "legacy_results_jsonl");
    assert_eq!(report["records_without_context"], 30);
    assert!(
        report["cross_context_comparison"]
            .as_str()
            .is_some_and(|c| c.starts_with("LegacyNoContext"))
    );
}

#[test]
fn a_context_recorded_study_is_analysed_strictly() {
    let dir = scratch("ctx-analysis");
    let m = manifest(&dir);
    let records = schedule_records(&m, Some("ctx-a"), standard_time);
    let report = analyze(&m, &write_legacy(&dir, &records)).expect("analysis");
    assert_eq!(report["context_sha256"], "ctx-a");
    assert_eq!(report["records_without_context"], 0);
    assert!(
        report["cross_context_comparison"]
            .as_str()
            .is_some_and(|c| c.starts_with("strict"))
    );
}

#[test]
fn analysis_refuses_foreign_records_and_reports_insufficient_legal_candidates() {
    let dir = scratch("insufficient");
    let mut m = manifest(&dir);
    let mut foreign = schedule_records(&m, None, standard_time);
    foreign[0].manifest_sha256 = "0000".into();
    assert!(
        analyze(&m, &write_legacy(&dir, &foreign)).is_err(),
        "another manifest's records are refused"
    );
    m.candidates.truncate(2);
    let records = schedule_records(&m, None, standard_time);
    let report = analyze(&m, &write_legacy(&dir, &records)).expect("analysis");
    assert!(report["ranking"].is_null());
    assert!(
        report["ranking_note"]
            .as_str()
            .is_some_and(|n| n.starts_with("InsufficientLegalCandidateEvidence"))
    );
}

#[test]
fn a_partial_prefix_study_and_invalid_slots_are_analysed_without_inventing_effects() {
    let dir = scratch("partial");
    let m = manifest(&dir);
    let all = schedule_records(&m, None, |b, arm| {
        if arm == "split" {
            None
        } else {
            standard_time(b, arm)
        }
    });
    let report = analyze(&m, &write_legacy(&dir, &all)).expect("analysis");
    assert!(
        report["effects"].get("split").is_none(),
        "no valid pair, no effect row"
    );
    assert_eq!(report["invalid_slots"], 6);
    let prefix = schedule_records(&m, None, standard_time)[..10].to_vec();
    let report = analyze(&m, &write_legacy(&dir, &prefix)).expect("prefix");
    assert!(report["effects"].as_object().is_some());
}

#[test]
fn too_few_independent_blocks_are_declared_inconclusive_with_no_interval() {
    let dir = scratch("fewblocks");
    let mut m = manifest(&dir);
    m.blocks = 2;
    m.training_blocks = 1;
    let records = schedule_records(&m, None, standard_time);
    let report = analyze(&m, &write_legacy(&dir, &records)).expect("analysis");
    let split = &report["effects"]["split"];
    assert_eq!(split["verdict"], "Inconclusive");
    assert_eq!(split["reason"], "InsufficientIndependentBlocks");
    assert!(split["ci95_percent"].is_null());
}

#[test]
fn a_candidate_outside_the_legal_space_is_a_verification_error_not_a_ranked_row() {
    let dir = scratch("illegal");
    let m = manifest(&dir);
    let d = m.digest();
    let mut records = schedule_records(&m, None, standard_time);
    for illegal in [
        "nice-19",
        "uclamp-max",
        "pm-qos-0",
        "gpu-power-profile",
        "scx-switch",
    ] {
        let mut bad = records.clone();
        bad.push(record(6, illegal, 1_000_000_000, &d));
        assert!(analyze(&m, &write_legacy(&dir, &bad)).is_err(), "{illegal}");
    }
    records[3].arm = "nice-19".into();
    assert!(
        analyze(&m, &write_legacy(&dir, &records)).is_err(),
        "an arm name outside the manifest"
    );
}

#[test]
fn bootstrap_is_deterministic_and_verdicts_follow_the_threshold() {
    let values = [10.0, 12.0, 9.0, 11.0, 10.5, 9.5];
    assert_eq!(bootstrap(&values, 7), bootstrap(&values, 7));
    let (low, high) = bootstrap(&values, 7);
    assert!(low > 2.0 && high < 15.0);
    assert_eq!(verdict(3.0, 9.0, 2.0), "Beneficial");
    assert_eq!(verdict(-9.0, -3.0, 2.0), "Harmful");
    assert_eq!(verdict(-1.0, 1.0, 2.0), "Equivalent");
    assert_eq!(verdict(-3.0, 4.0, 2.0), "Inconclusive");
}

#[test]
fn completion_markers_must_be_exactly_one_start_and_one_completion() {
    use super::run::elapsed_ns;
    let start = r#"{"phase":"WORKLOAD_START"}"#;
    let done = r#"{"phase":"WORKLOAD_COMPLETE","elapsed_ns":5}"#;
    assert_eq!(elapsed_ns(&format!("{start}\n{done}\n")).expect("ok"), 5);
    assert!(
        elapsed_ns(&format!("{start}\n")).is_err(),
        "no completion: a failed or incomplete build"
    );
    assert!(
        elapsed_ns(&format!("{start}\n{done}\n{done}\n")).is_err(),
        "duplicated completion"
    );
    assert!(
        elapsed_ns(&format!("{start}\n{start}\n{done}\n")).is_err(),
        "duplicated start"
    );
    assert!(
        elapsed_ns(&format!(
            "{start}\n{}\n",
            r#"{"phase":"WORKLOAD_COMPLETE"}"#
        ))
        .is_err(),
        "no elapsed"
    );
    assert!(elapsed_ns("not json\n").is_err());
}
