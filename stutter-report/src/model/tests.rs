use serde_json::json;
use stutter_core::ids::{CpuId, Pid, Tid};

use super::{ReportHeaderSummary, SpikePoint, WakeGraphEdge};

#[test]
fn report_header_typed_ids_preserve_json_numbers() {
    let header = ReportHeaderSummary {
        file_path: "summary.json".to_owned(),
        schema_version: 22,
        expected_schema_version: 22,
        run_name: "run".to_owned(),
        duration_ms: 1000,
        stop_reason: "completed".to_owned(),
        manual_pids: vec![Pid::new(1234)],
        tree_roots: vec![Pid::new(5678)],
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        event_stream_warning: None,
        watch_process: "game".to_owned(),
        persistent: false,
        csv_stream: "none".to_owned(),
        active_target_pids_count: 1,
    };

    let value = serde_json::to_value(&header).expect("serialize header");

    assert_eq!(value["manual_pids"], json!([1234]));
    assert_eq!(value["tree_roots"], json!([5678]));

    let decoded: ReportHeaderSummary =
        serde_json::from_value(value).expect("deserialize typed header");
    assert_eq!(decoded.manual_pids, vec![Pid::new(1234)]);
    assert_eq!(decoded.tree_roots, vec![Pid::new(5678)]);
}

#[test]
fn spike_point_typed_ids_preserve_json_numbers() {
    let point = SpikePoint {
        task: Tid::new(1234),
        class: "game".to_owned(),
        process_pid: Some(Pid::new(4321)),
        comm: "render".to_owned(),
        cpu: CpuId::new(2),
        wakeup_target_cpu: CpuId::new(3),
        latency_ns: 1_000_000,
        wakeup_ns: 1,
        switch_ns: 1_000_001,
        target_pending_wakeups: 0,
        observed_runnable_depth: 1,
        switch_prev_pid: Tid::new(0),
        switch_prev_state: 1,
        switch_prev_state_label: "S".to_owned(),
        scx_ops: None,
        primary_cause: None,
        cause_tags: Vec::new(),
    };

    let value = serde_json::to_value(&point).expect("serialize spike point");

    assert_eq!(value["task"], 1234);
    assert_eq!(value["process_pid"], 4321);
    assert_eq!(value["cpu"], 2);
    assert_eq!(value["wakeup_target_cpu"], 3);
    assert_eq!(value["switch_prev_pid"], 0);

    let decoded: SpikePoint = serde_json::from_value(value).expect("deserialize spike point");
    assert_eq!(decoded.task, Tid::new(1234));
    assert_eq!(decoded.process_pid, Some(Pid::new(4321)));
    assert_eq!(decoded.cpu, CpuId::new(2));
    assert_eq!(decoded.wakeup_target_cpu, CpuId::new(3));
    assert_eq!(decoded.switch_prev_pid, Tid::new(0));
}

#[test]
fn wake_graph_typed_ids_preserve_json_numbers() {
    let edge = WakeGraphEdge {
        waker_tid: Tid::new(1234),
        waker_comm: "waker".to_owned(),
        wakee_tid: Tid::new(5678),
        wakee_comm: "wakee".to_owned(),
        count: 2,
        max_latency_ns: 10_000,
    };

    let value = serde_json::to_value(&edge).expect("serialize wake graph");

    assert_eq!(value["waker_tid"], 1234);
    assert_eq!(value["wakee_tid"], 5678);

    let decoded: WakeGraphEdge = serde_json::from_value(value).expect("deserialize wake graph");
    assert_eq!(decoded.waker_tid, Tid::new(1234));
    assert_eq!(decoded.wakee_tid, Tid::new(5678));
}
