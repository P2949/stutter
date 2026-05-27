use tokio::sync::mpsc;

use super::*;
use crate::{
    alert::AlertPayload,
    recorder::{LiveRecorder, SpikeEvent},
    session_events::MonitorEvent,
};

#[test]
fn output_sink_registry_includes_only_active_optional_sinks() {
    let output = MonitorOutputConfig::default();
    let recorder = LiveRecorder::default();
    let registry = MonitorOutputSinkRegistry::for_runtime(output, &recorder, None);

    assert_eq!(registry.sink_names(), vec!["recorder", "stdout"]);
}

#[test]
fn output_sink_registry_includes_exporters_and_alert_when_active() {
    let output = MonitorOutputConfig {
        json_stream: true,
        verbose: false,
        ..MonitorOutputConfig::default()
    };
    let mut recorder = LiveRecorder::default();
    recorder.exporters.prometheus_state = Some(std::sync::Arc::new(
        crate::prometheus::PrometheusState::new_started_now(),
    ));
    recorder.stdout_spike_stream = Some(crate::recorder::StdoutJsonStream::new());

    let (alert_tx, _alert_rx) = mpsc::channel(1);
    let registry = MonitorOutputSinkRegistry::for_runtime(output, &recorder, Some(&alert_tx));
    let names = registry.sink_names();

    assert!(names.contains(&"recorder"));
    assert!(names.contains(&"prometheus"));
    assert!(names.contains(&"stdout"));
    assert!(names.contains(&"alert"));
    assert!(!names.contains(&"otel"));
}

#[test]
fn recorder_sink_stores_spike_in_buffer_when_stream_is_absent() {
    let mut recorder = LiveRecorder::default();
    recorder.buffers.spike_events = Some(crate::recorder::SpikeEventBuffer::with_max_events(10));

    let spike = SpikeEvent {
        elapsed_ms: Some(5),
        task: 10.into(),
        latency_ns: 2_000_000,
        ..Default::default()
    };

    let event = MonitorEvent::Spike {
        event: Box::new(spike),
    };

    let mut ctx = MonitorSinkContext {
        recorder: &mut recorder,
        alert_sender: None,
        output: MonitorOutputConfig::default(),
    };
    let mut sink = RecorderSink::new();
    sink.on_event(&event, &mut ctx).unwrap();

    assert_eq!(
        recorder
            .buffers
            .spike_events
            .as_ref()
            .unwrap()
            .as_slice()
            .len(),
        1
    );
}

#[test]
fn alert_sink_counts_full_channel_drops() {
    let mut recorder = LiveRecorder::default();
    let (tx, _rx) = mpsc::channel(1);
    // fill the channel
    let payload = AlertPayload {
        title: "title".to_owned(),
        message: "message".to_owned(),
        task: 1,
        active: true,
        class: crate::process_tree::TaskClass::Unknown,
        comm: "task".to_owned(),
        process_pid: None,
        process_comm: String::new(),
        latency_ns: 1_000_000,
        latency_ms: 1,
        cpu: 0,
        prio: 120,
        wakeup_ns: 1,
        switch_ns: 2,
        elapsed_ms: 3,
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
    };
    tx.try_send(payload.clone()).unwrap();

    let event = MonitorEvent::Alert {
        payload: Box::new(payload),
    };

    let mut ctx = MonitorSinkContext {
        recorder: &mut recorder,
        alert_sender: Some(&tx),
        output: MonitorOutputConfig::default(),
    };
    let mut sink = AlertSink::new();
    sink.on_event(&event, &mut ctx).unwrap();

    assert_eq!(recorder.counters.alert_events_dropped_count, 1);
}
