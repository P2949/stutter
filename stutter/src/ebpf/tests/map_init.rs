//! Tests for eBPF map initialization behavior without requiring eBPF privileges.

use std::collections::BTreeSet;

use crate::ebpf::map_init::{
    MapInitOps, initialize_ebpf_maps, map_init_context, missing_map_context,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FakeMap(&'static str);

#[derive(Default, Debug)]
struct FakeMapInitOps {
    missing_maps: BTreeSet<&'static str>,
    init_failures: BTreeSet<&'static str>,
    async_fd_failure: bool,
    calls: Vec<&'static str>,
}

impl FakeMapInitOps {
    fn missing(maps: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            missing_maps: maps.into_iter().collect(),
            ..Self::default()
        }
    }

    fn init_failures(maps: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            init_failures: maps.into_iter().collect(),
            ..Self::default()
        }
    }

    fn with_async_fd_failure() -> Self {
        Self {
            async_fd_failure: true,
            ..Self::default()
        }
    }

    fn required_map(&mut self, name: &'static str) -> anyhow::Result<FakeMap> {
        self.calls.push(name);

        if self.missing_maps.contains(name) {
            anyhow::bail!("{}", missing_map_context(name));
        }

        if self.init_failures.contains(name) {
            anyhow::bail!("{}", map_init_context(name));
        }

        Ok(FakeMap(name))
    }

    fn optional_map(&mut self, name: &'static str) -> anyhow::Result<Option<FakeMap>> {
        self.calls.push(name);

        if self.missing_maps.contains(name) {
            return Ok(None);
        }

        if self.init_failures.contains(name) {
            anyhow::bail!("{}", map_init_context(name));
        }

        Ok(Some(FakeMap(name)))
    }
}

impl MapInitOps for FakeMapInitOps {
    type TargetPidMap = FakeMap;
    type TargetIrqMap = FakeMap;
    type DropCounters = FakeMap;
    type Events = FakeMap;
    type PrevFaultsMap = FakeMap;

    fn target_pid_map(&mut self) -> anyhow::Result<Self::TargetPidMap> {
        self.required_map("TARGET_PIDS")
    }

    fn target_irq_map(&mut self) -> anyhow::Result<Option<Self::TargetIrqMap>> {
        self.optional_map("TARGET_IRQS")
    }

    fn drop_counters(&mut self) -> anyhow::Result<Self::DropCounters> {
        self.required_map("DROP_COUNTERS")
    }

    fn events(&mut self) -> anyhow::Result<Self::Events> {
        let events = self.required_map("EVENTS")?;

        if self.async_fd_failure {
            anyhow::bail!("eBPF load failed: events ringbuf async fd");
        }

        Ok(events)
    }

    fn prev_faults_map(&mut self) -> anyhow::Result<Option<Self::PrevFaultsMap>> {
        self.optional_map("PREV_FAULTS")
    }
}

#[test]
fn map_initialization_loads_expected_maps_in_order() {
    let mut ops = FakeMapInitOps::default();

    let maps = initialize_ebpf_maps(&mut ops).unwrap();

    assert_eq!(
        ops.calls,
        vec![
            "TARGET_PIDS",
            "TARGET_IRQS",
            "DROP_COUNTERS",
            "EVENTS",
            "PREV_FAULTS",
        ]
    );
    assert_eq!(maps.target_pid_map, FakeMap("TARGET_PIDS"));
    assert_eq!(maps.target_irq_map, Some(FakeMap("TARGET_IRQS")));
    assert_eq!(maps.drop_counters, FakeMap("DROP_COUNTERS"));
    assert_eq!(maps.events, FakeMap("EVENTS"));
    assert_eq!(maps.prev_faults_map, Some(FakeMap("PREV_FAULTS")));
}

#[test]
fn missing_required_target_pids_map_fails_with_missing_map_context() {
    let mut ops = FakeMapInitOps::missing(["TARGET_PIDS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: TARGET_PIDS map not found"));
    assert_eq!(ops.calls, vec!["TARGET_PIDS"]);
}

#[test]
fn missing_required_drop_counters_map_fails_with_missing_map_context() {
    let mut ops = FakeMapInitOps::missing(["DROP_COUNTERS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: DROP_COUNTERS map not found"));
    assert_eq!(
        ops.calls,
        vec!["TARGET_PIDS", "TARGET_IRQS", "DROP_COUNTERS"]
    );
}

#[test]
fn missing_required_events_map_fails_with_missing_map_context() {
    let mut ops = FakeMapInitOps::missing(["EVENTS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: EVENTS map not found"));
    assert_eq!(
        ops.calls,
        vec!["TARGET_PIDS", "TARGET_IRQS", "DROP_COUNTERS", "EVENTS"]
    );
}

#[test]
fn missing_optional_maps_are_absent_not_errors() {
    let mut ops = FakeMapInitOps::missing(["TARGET_IRQS", "PREV_FAULTS"]);

    let maps = initialize_ebpf_maps(&mut ops).unwrap();

    assert_eq!(maps.target_irq_map, None);
    assert_eq!(maps.prev_faults_map, None);
    assert_eq!(
        ops.calls,
        vec![
            "TARGET_PIDS",
            "TARGET_IRQS",
            "DROP_COUNTERS",
            "EVENTS",
            "PREV_FAULTS",
        ]
    );
}

#[test]
fn optional_target_irq_type_failure_reports_map_init_context() {
    let mut ops = FakeMapInitOps::init_failures(["TARGET_IRQS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: TARGET_IRQS map init"));
    assert_eq!(ops.calls, vec!["TARGET_PIDS", "TARGET_IRQS"]);
}

#[test]
fn optional_prev_faults_type_failure_reports_map_init_context() {
    let mut ops = FakeMapInitOps::init_failures(["PREV_FAULTS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: PREV_FAULTS map init"));
    assert_eq!(
        ops.calls,
        vec![
            "TARGET_PIDS",
            "TARGET_IRQS",
            "DROP_COUNTERS",
            "EVENTS",
            "PREV_FAULTS",
        ]
    );
}

#[test]
fn required_drop_counters_type_failure_reports_map_init_context() {
    let mut ops = FakeMapInitOps::init_failures(["DROP_COUNTERS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: DROP_COUNTERS map init"));
    assert_eq!(
        ops.calls,
        vec!["TARGET_PIDS", "TARGET_IRQS", "DROP_COUNTERS"]
    );
}

#[test]
fn events_ringbuf_type_failure_reports_map_init_context() {
    let mut ops = FakeMapInitOps::init_failures(["EVENTS"]);

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: EVENTS map init"));
    assert_eq!(
        ops.calls,
        vec!["TARGET_PIDS", "TARGET_IRQS", "DROP_COUNTERS", "EVENTS"]
    );
}

#[test]
fn events_async_fd_failure_reports_async_fd_context() {
    let mut ops = FakeMapInitOps::with_async_fd_failure();

    let err = initialize_ebpf_maps(&mut ops).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("eBPF load failed: events ringbuf async fd"));
    assert_eq!(
        ops.calls,
        vec!["TARGET_PIDS", "TARGET_IRQS", "DROP_COUNTERS", "EVENTS"]
    );
}
