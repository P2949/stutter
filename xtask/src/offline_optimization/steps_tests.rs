use std::{path::PathBuf, time::Duration};

use super::{steps::*, *};

fn plan(kind: Kind) -> Plan {
    Plan {
        kind,
        source_revision: "a".repeat(40),
        compiler: "clang".into(),
        compiler_version: "19".into(),
        flags: vec!["-O2".into()],
        training_inputs: vec![
            InputRef {
                path: "train/a".into(),
                sha256: "1".into(),
            },
            InputRef {
                path: "train/b".into(),
                sha256: "2".into(),
            },
        ],
        test_inputs: vec![InputRef {
            path: "test/a".into(),
            sha256: "3".into(),
        }],
        input_binary: PathBuf::from("/work/app"),
        output_binary: PathBuf::from("/work/app.optimized"),
        profile: Some(ProfileRef {
            path: "app.prof".into(),
            sha256: "4".into(),
            build_id: "abcd".into(),
        }),
        build_argv: ["clang", "main.c", "-o", "{output}"]
            .map(Into::into)
            .to_vec(),
        run_argv: ["{binary}", "{input}"].map(Into::into).to_vec(),
        verify_argv: ["{binary}", "--selftest"].map(Into::into).to_vec(),
        collect_profile: false,
        guardrails: Guardrails {
            max_startup_regression_percent: 5.0,
            max_size_growth_percent: 20.0,
            max_build_time_ratio: 3.0,
        },
    }
}

fn names(steps: &[Step]) -> Vec<&str> {
    steps.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn each_kind_yields_its_ordered_steps_with_every_artifact_under_the_work_directory() {
    let work = PathBuf::from("/tmp/w");
    let pgo = workflow_steps(&plan(Kind::Pgo), &work).unwrap();
    assert_eq!(
        names(&pgo),
        [
            "build-instrumented",
            "train-0",
            "train-1",
            "merge-profile",
            "rebuild",
            "verify",
            "compare-0"
        ]
    );
    assert!(pgo[0].argv.contains(&"/tmp/w/instrumented".to_owned()));
    assert!(
        pgo[0]
            .argv
            .contains(&"-fprofile-generate=/tmp/w/raw".to_owned())
    );
    assert_eq!(pgo[1].argv, ["/tmp/w/instrumented", "train/a"]);
    assert_eq!(pgo[1].env[0].0, "LLVM_PROFILE_FILE");
    assert!(pgo[4].argv.contains(&"/work/app.optimized".to_owned()));
    assert_eq!(pgo[5].argv, ["/work/app.optimized", "--selftest"]);
    assert_eq!(pgo[6].argv[0], "hyperfine");

    let fdo = workflow_steps(&plan(Kind::AutoFdo), &work).unwrap();
    assert_eq!(
        names(&fdo),
        ["convert-profile", "rebuild", "verify", "compare-0"]
    );
    let bolt = workflow_steps(&plan(Kind::Bolt), &work).unwrap();
    assert_eq!(
        names(&bolt),
        ["convert-profile", "bolt", "verify", "compare-0"]
    );
    assert_eq!(bolt[1].argv[0], "llvm-bolt");
}

#[test]
fn a_plan_missing_a_command_or_with_an_unplain_path_makes_no_steps() {
    let work = PathBuf::from("/tmp/w");
    let mut no_build = plan(Kind::Pgo);
    no_build.build_argv.clear();
    assert!(workflow_steps(&no_build, &work).is_err());
    let mut no_verify = plan(Kind::Pgo);
    no_verify.verify_argv.clear();
    assert!(workflow_steps(&no_verify, &work).is_err());
    let mut spaced = plan(Kind::Pgo);
    spaced.test_inputs[0].path = "test/a b".into();
    assert!(workflow_steps(&spaced, &work).is_err());
    let mut no_profile = plan(Kind::Bolt);
    no_profile.profile = None;
    assert!(workflow_steps(&no_profile, &work).is_err());
}

fn step(name: &str, argv: &[&str]) -> Step {
    Step {
        name: name.into(),
        argv: argv.iter().map(|s| (*s).into()).collect(),
        env: Vec::new(),
    }
}

#[test]
fn steps_run_in_order_and_the_first_failure_stops_the_rest() {
    let work = std::env::temp_dir().join(format!("stutter-oo-{}", std::process::id()));
    let steps = [
        step("a", &["true"]),
        step("b", &["false"]),
        step("c", &["true"]),
    ];
    let record = run_steps(
        &steps,
        &work,
        std::path::Path::new("/x/out"),
        Duration::from_secs(30),
    )
    .unwrap();
    assert_eq!(record.steps[0].1, StepResult::Passed);
    assert!(matches!(record.steps[1].1, StepResult::Failed(_)));
    assert_eq!(record.steps[2].1, StepResult::NotRun);
    assert!(!record.all_passed());
    std::fs::remove_dir_all(&work).unwrap();
}

#[test]
fn a_missing_program_and_a_timeout_fail_the_step_and_all_passing_is_all_passed() {
    let work = std::env::temp_dir().join(format!("stutter-oo2-{}", std::process::id()));
    let missing = [step("m", &["/nonexistent-stutter-tool"])];
    let record = run_steps(
        &missing,
        &work,
        std::path::Path::new("/x"),
        Duration::from_secs(30),
    )
    .unwrap();
    assert!(
        matches!(&record.steps[0].1, StepResult::Failed(reason) if reason.contains("could not start"))
    );

    let slow = [step("s", &["sleep", "30"])];
    let record = run_steps(
        &slow,
        &work,
        std::path::Path::new("/x"),
        Duration::from_millis(300),
    )
    .unwrap();
    assert_eq!(record.steps[0].1, StepResult::Failed("timed out".into()));

    let ok = [step("ok", &["true"]), step("ok2", &["true"])];
    assert!(
        run_steps(
            &ok,
            &work,
            std::path::Path::new("/x"),
            Duration::from_secs(30)
        )
        .unwrap()
        .all_passed()
    );
    std::fs::remove_dir_all(&work).unwrap();
}

#[test]
fn collecting_a_profile_adds_a_perf_record_step_on_the_one_training_input() {
    let work = PathBuf::from("/tmp/w");
    let mut fdo = plan(Kind::AutoFdo);
    fdo.collect_profile = true;
    fdo.profile = None;
    fdo.training_inputs.truncate(1);
    let steps = workflow_steps(&fdo, &work).unwrap();
    assert_eq!(
        names(&steps),
        [
            "collect-profile",
            "convert-profile",
            "rebuild",
            "verify",
            "compare-0"
        ]
    );
    assert_eq!(
        steps[0].argv,
        [
            "perf",
            "record",
            "-b",
            "-o",
            "/tmp/w/perf.data",
            "--",
            "/work/app",
            "train/a"
        ]
    );
    assert!(
        steps[1]
            .argv
            .contains(&"--profile=/tmp/w/perf.data".to_owned())
    );

    let mut bolt = plan(Kind::Bolt);
    bolt.collect_profile = true;
    bolt.profile = None;
    bolt.training_inputs.truncate(1);
    let steps = workflow_steps(&bolt, &work).unwrap();
    assert!(steps[0].argv.contains(&"cycles:u".to_owned()));
    assert!(steps[1].argv.contains(&"/tmp/w/perf.data".to_owned()));
}

#[test]
fn collection_conflicting_with_an_imported_profile_or_many_inputs_is_a_finding() {
    let all: std::collections::BTreeSet<String> = [
        "clang",
        "perf",
        "create_llvm_prof",
        "perf2bolt",
        "llvm-bolt",
    ]
    .map(String::from)
    .into();
    let same = |_: &std::path::Path| Some("abcd".to_owned());
    let mut both = plan(Kind::Bolt);
    both.collect_profile = true;
    both.training_inputs.truncate(1);
    assert_eq!(
        validate(&both, &all, same),
        vec![Finding::ProfileAndCollection]
    );
    let mut many = plan(Kind::Bolt);
    many.collect_profile = true;
    many.profile = None;
    assert_eq!(
        validate(&many, &all, same),
        vec![Finding::CollectNeedsOneTrainingInput]
    );
    let mut ok = plan(Kind::Bolt);
    ok.collect_profile = true;
    ok.profile = None;
    ok.training_inputs.truncate(1);
    assert!(validate(&ok, &all, same).is_empty());
}
