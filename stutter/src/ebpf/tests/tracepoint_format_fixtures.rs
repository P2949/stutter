use std::path::{Path, PathBuf};

use stutter_common::tracepoint_offsets::{
    CPU_FREQUENCY_FIELDS, IRQ_HANDLER_FIELDS, SCHED_MIGRATE_TASK_FIELDS, SCHED_STAT_WAIT_FIELDS,
    SCHED_SWITCH_FIELDS, SCHED_WAKEUP_FIELDS, TRACEPOINT_CPU_FREQUENCY,
    TRACEPOINT_IRQ_HANDLER_ENTRY, TRACEPOINT_IRQ_HANDLER_EXIT, TRACEPOINT_SCHED_MIGRATE_TASK,
    TRACEPOINT_SCHED_STAT_WAIT, TRACEPOINT_SCHED_SWITCH, TRACEPOINT_SCHED_WAKEUP,
    TRACEPOINT_SCHED_WAKEUP_NEW, TracepointFieldSpec, TracepointName,
};

use super::*;

struct TracepointFixture {
    name: &'static str,
}

struct TracepointFixtureCase {
    relative_path: &'static str,
    tracepoint_name: TracepointName<'static>,
    expected_fields: &'static [TracepointFieldSpec],
}

const TRACEPOINT_FIXTURES: &[TracepointFixture] = &[TracepointFixture {
    name: "linux_6_12_reference",
}];

const TRACEPOINT_FIXTURE_CASES: &[TracepointFixtureCase] = &[
    TracepointFixtureCase {
        relative_path: "sched/sched_wakeup/format",
        tracepoint_name: TRACEPOINT_SCHED_WAKEUP,
        expected_fields: SCHED_WAKEUP_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "sched/sched_wakeup_new/format",
        tracepoint_name: TRACEPOINT_SCHED_WAKEUP_NEW,
        expected_fields: SCHED_WAKEUP_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "sched/sched_switch/format",
        tracepoint_name: TRACEPOINT_SCHED_SWITCH,
        expected_fields: SCHED_SWITCH_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "sched/sched_migrate_task/format",
        tracepoint_name: TRACEPOINT_SCHED_MIGRATE_TASK,
        expected_fields: SCHED_MIGRATE_TASK_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "power/cpu_frequency/format",
        tracepoint_name: TRACEPOINT_CPU_FREQUENCY,
        expected_fields: CPU_FREQUENCY_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "sched/sched_stat_wait/format",
        tracepoint_name: TRACEPOINT_SCHED_STAT_WAIT,
        expected_fields: SCHED_STAT_WAIT_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "irq/irq_handler_entry/format",
        tracepoint_name: TRACEPOINT_IRQ_HANDLER_ENTRY,
        expected_fields: IRQ_HANDLER_FIELDS,
    },
    TracepointFixtureCase {
        relative_path: "irq/irq_handler_exit/format",
        tracepoint_name: TRACEPOINT_IRQ_HANDLER_EXIT,
        expected_fields: IRQ_HANDLER_FIELDS,
    },
];

fn tracepoint_fixture_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tracepoints")
        .join(name)
}

#[test]
fn known_kernel_tracepoint_format_fixtures_match_expected_offsets() {
    for fixture in TRACEPOINT_FIXTURES {
        let root = tracepoint_fixture_root(fixture.name);
        assert!(
            root.exists(),
            "missing tracepoint fixture root {}",
            root.display()
        );

        for case in TRACEPOINT_FIXTURE_CASES {
            let path = root.join(case.relative_path);
            let format = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

            validate_tracepoint_format_named(case.tracepoint_name, &format, case.expected_fields)
                .unwrap_or_else(|err| {
                    panic!(
                        "tracepoint fixture {} {} failed expected offset validation: {err:#}",
                        fixture.name, case.relative_path
                    )
                });
        }
    }
}

#[test]
fn known_kernel_tracepoint_fixtures_pass_preflight() {
    for fixture in TRACEPOINT_FIXTURES {
        let root = tracepoint_fixture_root(fixture.name);
        let report = tracepoint_preflight(&root, true, true, true, true, true);

        assert!(
            report.errors.is_empty(),
            "fixture {} should not have required preflight errors: {report:#?}",
            fixture.name
        );

        assert_eq!(report.sched_wakeup, "ok");
        assert_eq!(report.sched_switch, "ok");
        assert_eq!(report.sched_wakeup_new, "ok");
        assert_eq!(report.sched_migrate_task, "ok");
        assert_eq!(report.cpu_frequency, "ok");
        assert_eq!(report.sched_stat_wait, "ok");
        assert_eq!(report.irq_handler, "ok");
        assert_eq!(report.block_rq, "ok");
    }
}
