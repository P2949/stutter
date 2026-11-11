use std::{collections::BTreeSet, path::PathBuf};

use super::{
    analyze::{analyze, bootstrap, verdict},
    manifest::{Arm, Manifest, Placement, effective_cpus_of_status, parse_cpulist},
    run::{
        Readback, Record, competing_game, ensure_same_context, parse_readback, placed_command,
        readback_matches,
    },
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owned-study-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn allowed() -> BTreeSet<u32> {
    (0..12).collect()
}

fn identity(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("identity.json");
    let text = serde_json::json!({"kind":"compile","source_tree":format!("git:{}", "a".repeat(40)),
        "target":"t","build_state":"cold","cache_policy":"none","job_count":2,
        "command":["cargo","build"],"working_directory":"/tmp/x","toolchain":"t","environment":"e"});
    std::fs::write(&path, text.to_string()).expect("identity");
    path
}

fn arm(name: &str, compile: Placement, hogs: Placement) -> Arm {
    Arm {
        name: name.into(),
        compile,
        hogs,
    }
}

fn manifest(dir: &std::path::Path) -> Manifest {
    Manifest {
        schema_version: 1,
        study_id: "demo".into(),
        identity: identity(dir),
        hog_threads: 12,
        blocks: 6,
        training_blocks: 3,
        arm_timeout_secs: 120,
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

#[test]
fn a_competing_game_is_found_but_a_shell_mentioning_one_is_not() {
    let root = scratch("proc");
    let add = |pid: &str, comm: &str, cmdline: &str| {
        let dir = root.join(pid);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("comm"), comm).expect("comm");
        std::fs::write(dir.join("cmdline"), cmdline.replace(' ', "\0")).expect("cmdline");
    };
    add("10", "zsh", "zsh -c grep gamescope");
    add("11", "stutter", "stutter run");
    assert!(!competing_game(&root, 1));
    add("12", "game.exe", "proton steamapps/common/Game/game.exe");
    assert!(competing_game(&root, 1));
    assert!(
        !competing_game(&root, 12),
        "the caller's own pid is excluded"
    );
    assert!(
        competing_game(&root.join("missing"), 1),
        "an unreadable /proc is not a safe host"
    );
}

fn record(block: u32, arm: &str, ns: u64, digest: &str) -> Record {
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
    }
}

fn write_results(dir: &std::path::Path, records: &[Record]) -> PathBuf {
    let path = dir.join("results.jsonl");
    let text: String = records
        .iter()
        .map(|r| serde_json::to_string(r).expect("json") + "\n")
        .collect();
    std::fs::write(&path, text).expect("results");
    path
}

#[test]
fn analysis_classifies_effects_and_ranks_only_legal_candidates() {
    let dir = scratch("analysis");
    let m = manifest(&dir);
    let d = m.digest();
    let mut records = Vec::new();
    for block in 1..=6u32 {
        let wobble = u64::from(block % 2) * 20_000_000;
        records.push(record(block, "baseline", 8_000_000_000, &d));
        records.push(record(block, "compile-0-5", 8_000_000_000 + wobble, &d));
        records.push(record(block, "hogs-6-11", 6_000_000_000 + wobble, &d));
        records.push(record(block, "split", 5_000_000_000 + wobble, &d));
        records.push(record(block, "starved", 12_000_000_000 + wobble, &d));
    }
    let report = analyze(&m, &write_results(&dir, &records)).expect("analysis");
    assert_eq!(report["effects"]["hogs-6-11"]["verdict"], "Beneficial");
    assert_eq!(report["effects"]["split"]["verdict"], "Beneficial");
    assert_eq!(report["effects"]["starved"]["verdict"], "Harmful");
    assert_eq!(report["effects"]["compile-0-5"]["verdict"], "Equivalent");
    assert_eq!(report["training_favourite"]["candidate"], "split");
    assert_eq!(report["ranking"]["top1_agrees"], true);
    assert_eq!(report["authorizes_mutation"], false);
    assert_eq!(report["outside_space_dropped"], 0);
}

#[test]
fn analysis_refuses_foreign_records_and_reports_insufficient_evidence() {
    let dir = scratch("insufficient");
    let m = manifest(&dir);
    let d = m.digest();
    let foreign = write_results(&dir, &[record(1, "baseline", 1, "0000")]);
    assert!(
        analyze(&m, &foreign).is_err(),
        "another manifest's records are refused"
    );
    let mut records = Vec::new();
    for block in 1..=6u32 {
        records.push(record(block, "baseline", 8_000_000_000, &d));
        records.push(record(block, "split", 6_000_000_000, &d));
    }
    let report = analyze(&m, &write_results(&dir, &records)).expect("analysis");
    assert!(report["ranking"].is_null());
    assert!(
        report["ranking_note"]
            .as_str()
            .is_some_and(|n| n.starts_with("InsufficientLegalCandidateEvidence"))
    );
    let mut invalid = record(1, "split", 1, &d);
    invalid.valid = false;
    let partial = write_results(&dir, &[record(1, "baseline", 8_000_000_000, &d), invalid]);
    assert!(
        analyze(&m, &partial)
            .expect("partial")
            .get("effects")
            .is_some_and(|e| e.as_object().is_some_and(|o| o.is_empty())),
        "an invalid slot yields no effect"
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
fn a_resumed_study_repeats_no_recorded_slot() {
    let dir = scratch("resume");
    let m = manifest(&dir);
    let d = m.digest();
    let all = m.schedule();
    let recorded: String = all[..7]
        .iter()
        .map(|s| {
            serde_json::to_string(&record(s.block, &m.arms()[s.arm].name, 1, &d)).expect("json")
                + "\n"
        })
        .collect();
    let pending = super::run::pending_slots(&m, &recorded);
    assert_eq!(pending.len(), all.len() - 7);
    assert_eq!(pending[..], all[7..]);
    assert!(
        super::run::pending_slots(&m, "{not json}\n").len() == all.len(),
        "an unreadable line records nothing"
    );
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

#[test]
fn a_corrupted_results_file_authorizes_nothing() {
    let dir = scratch("corrupt");
    let m = manifest(&dir);
    let path = dir.join("results.jsonl");
    std::fs::write(&path, "{\"slot\": 1\n").expect("write");
    assert!(analyze(&m, &path).is_err());
    assert!(analyze(&m, &dir.join("absent.jsonl")).is_err());
}

#[test]
fn records_from_another_context_are_never_mixed_or_resumed() {
    let dir = scratch("context");
    let m = manifest(&dir);
    let d = m.digest();
    let mut a = record(1, "baseline", 8_000_000_000, &d);
    let mut b = record(1, "split", 6_000_000_000, &d);
    a.context = Some("ctx-a".into());
    b.context = Some("ctx-b".into());
    assert!(
        analyze(&m, &write_results(&dir, &[a.clone(), b.clone()])).is_err(),
        "two contexts in one study"
    );
    b.context = Some("ctx-a".into());
    let report = analyze(&m, &write_results(&dir, &[a.clone(), b])).expect("one context");
    assert_eq!(report["context_sha256"], "ctx-a");
    assert_eq!(report["records_without_context"], 0);
    let line = serde_json::to_string(&a).expect("json");
    assert!(ensure_same_context(&line, "ctx-a").is_ok());
    assert!(
        ensure_same_context(&line, "ctx-b").is_err(),
        "a changed boot/kernel/scheduler refuses the resume"
    );
    assert!(ensure_same_context("", "ctx-b").is_ok());
    let legacy = serde_json::to_string(&record(1, "baseline", 1, &d)).expect("json");
    assert!(
        ensure_same_context(&legacy, "x").is_ok(),
        "a record with no context is reported, not silently trusted"
    );
    assert_eq!(
        super::manifest::context_digest(&m.identity).expect("digest"),
        super::manifest::context_digest(&m.identity).expect("digest")
    );
}

#[test]
fn an_illegal_candidate_with_a_great_score_never_reaches_the_ranker() {
    let dir = scratch("illegal");
    let m = manifest(&dir);
    let d = m.digest();
    let mut records = Vec::new();
    for block in 1..=6u32 {
        records.push(record(block, "baseline", 8_000_000_000, &d));
        records.push(record(block, "hogs-6-11", 7_000_000_000, &d));
        records.push(record(block, "split", 6_900_000_000, &d));
        records.push(record(block, "starved", 12_000_000_000, &d));
        // Imported or replayed evidence for operations outside the legal space, with the best score of all.
        for illegal in [
            "nice-19",
            "uclamp-max",
            "pm-qos-0",
            "gpu-power-profile",
            "scx-switch",
        ] {
            records.push(record(block, illegal, 1_000_000_000, &d));
        }
    }
    let report = analyze(&m, &write_results(&dir, &records)).expect("analysis");
    let effects = report["effects"].as_object().expect("effects");
    assert!(
        effects
            .keys()
            .all(|name| m.candidates.iter().any(|c| &c.name == name))
    );
    assert!(effects.get("nice-19").is_none());
    assert_ne!(report["training_favourite"]["candidate"], "nice-19");
    assert_eq!(report["authorizes_mutation"], false);
}
