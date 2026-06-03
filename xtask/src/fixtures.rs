use crate::workflow::{CommandSpec, WorkflowSpec};

pub const SCHEMA_CHECK_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["test", "-p", "stutter", "artifact_contract_tests"],
}];

pub const FIXTURE_CHECK_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["test", "-p", "stutter", "validation_corpus"],
}];

pub const FIXTURE_UPDATE_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "cargo",
        args: &[
            "test",
            "-p",
            "stutter",
            "validation_corpus_tests::regenerate::regenerate_validation_corpus",
            "--",
            "--ignored",
            "--exact",
        ],
    },
    CommandSpec {
        program: "cargo",
        args: &[
            "test",
            "-p",
            "stutter",
            "validation_corpus_tests::regenerate::regenerate_public_examples_v23",
            "--",
            "--ignored",
            "--exact",
        ],
    },
];

pub const REPORT_GOLDEN_UPDATE_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &[
        "test",
        "-p",
        "stutter",
        "report::tests::report_text_rendering_matches_snapshot_fixture",
        "--",
        "--exact",
    ],
}];

pub const SCHEMA_CHECK_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "schema-check",
    description: "validates artifact contract tests and public example artifact schema expectations",
    affected_paths: &[
        "stutter/src/artifact_contract_tests.rs",
        "docs/examples/artifacts/v23/**",
    ],
    commands: SCHEMA_CHECK_COMMANDS,
};

pub const FIXTURE_CHECK_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "fixture-check",
    description: "validates committed validation corpus fixtures and fixture metadata",
    affected_paths: &[
        "stutter/src/validation_corpus_tests/",
        "stutter/tests/fixtures/runs/**",
    ],
    commands: FIXTURE_CHECK_COMMANDS,
};

pub const FIXTURE_UPDATE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "fixture-update",
    description: "updates validation corpus fixtures and public v23 example artifact fixtures",
    affected_paths: &[
        "stutter/tests/fixtures/runs/**",
        "docs/examples/artifacts/v23/**",
    ],
    commands: FIXTURE_UPDATE_COMMANDS,
};

pub const REPORT_GOLDEN_UPDATE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "report-golden-update",
    description: "validates the committed report text golden output fixture",
    affected_paths: &[
        "stutter/src/report/snapshots/text_report_minimal.snap",
        "stutter/src/report/mod.rs",
    ],
    commands: REPORT_GOLDEN_UPDATE_COMMANDS,
};
