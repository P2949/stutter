use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::{metrics::TaskStats, process_tree::TaskClass};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct FocusEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub old_kind: Option<String>,
    pub kind: Option<String>,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub confidence: f32,
    pub score: f32,
    pub situation: Option<crate::autotune::state::SituationKind>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct TreeEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
    pub comm: String,
    pub process_comm: std::sync::Arc<str>,
    pub class: TaskClass,
    pub from_cgroup: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SpikeEvent {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub comm: String,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct MigrationEventRecord {
    pub elapsed_ms: u64,
    pub tid: u32,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct CpuFreqRecord {
    pub elapsed_ms: u64,
    pub cpu: u32,
    pub freq_khz: u32,
    pub timestamp_ns: u64,
}

pub struct SpikeDiagnosticContext {
    pub scx_ops: Option<String>,
    pub scx_state: Option<String>,
    pub scx_enable_seq: Option<String>,
    pub cause_tags: Vec<String>,
    pub primary_cause: Option<String>,
    pub waker_tid: u32,
    pub waker_comm: String,
}

impl SpikeEvent {
    pub fn from_task_stats(
        monotonic_start_ns: Option<u64>,
        stats: &TaskStats,
        event: &SchedulerEvent,
        fault_deltas: (u64, u64),
        diag: SpikeDiagnosticContext,
    ) -> Self {
        Self {
            elapsed_ms: crate::recorder::session::elapsed_ms_from_monotonic(
                monotonic_start_ns,
                event.switch_ns,
            ),
            task: event.tid,
            active: stats.active,
            class: stats.class,
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.clone(),
            comm: stats.comm.clone(),
            cpu: event.cpu,
            wakeup_target_cpu: event.wakeup_target_cpu,
            prio: event.prio,
            latency_ns: event.latency_ns,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            switch_prev_pid: event.switch_prev_pid,
            switch_prev_state: event.switch_prev_state,
            switch_prev_state_label: crate::sched_state::classify_switch_prev_state(
                event.switch_prev_state,
            )
            .to_owned(),
            waker_tid: diag.waker_tid,
            waker_comm: diag.waker_comm,
            target_pending_wakeups: event.target_pending_wakeups,
            observed_runnable_depth: event.observed_runnable_depth,
            major_faults: fault_deltas.0,
            minor_faults: fault_deltas.1,
            scx_ops: diag.scx_ops,
            scx_state: diag.scx_state,
            scx_enable_seq: diag.scx_enable_seq,
            cause_tags: diag.cause_tags,
            primary_cause: diag.primary_cause,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct IrqEventRecord {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    pub irq: u32,
    pub cpu: u32,
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct GpuSample {
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drm_card: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_node: Option<String>,
    pub gpu_busy_percent: Option<u32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_percent: Option<u32>,
    pub gpu_clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub temp_millidegrees: Option<u32>,
    pub power_microwatts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_limit_reason: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct FrameEvent {
    pub elapsed_ms: u64,
    pub frametime_ms: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct BlockIoRecord {
    pub elapsed_ms: u64,
    pub tid: u32,
    #[serde(default = "default_block_io_correlation_basis")]
    pub correlation_basis: Cow<'static, str>,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: String,
}

fn default_block_io_correlation_basis() -> Cow<'static, str> {
    Cow::Borrowed("dev+sector")
}

pub(crate) fn default_block_io_correlation_basis_string() -> String {
    default_block_io_correlation_basis().into_owned()
}

pub(crate) fn default_block_io_correlation_confidence_string() -> String {
    crate::ebpf_loader::BlockIoCorrelationBasis::DevSector
        .confidence()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_event_serializes_expected_fields() {
        let event = FocusEvent {
            elapsed_ms: 1234,
            action: "changed".to_owned(),
            old_kind: Some("Browser".to_owned()),
            kind: Some("Game".to_owned()),
            root_pids: vec![10],
            member_pids: vec![10, 11],
            confidence: 0.75,
            score: 0.82,
            situation: Some(crate::autotune::state::SituationKind::GameFocused),
            reasons: vec!["focus resolver selected game".to_owned()],
        };

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"elapsed_ms\":1234"));
        assert!(json.contains("\"action\":\"changed\""));
        assert!(json.contains("\"old_kind\":\"Browser\""));
        assert!(json.contains("\"kind\":\"Game\""));
        assert!(json.contains("\"root_pids\":[10]"));
        assert!(json.contains("\"member_pids\":[10,11]"));
        assert!(json.contains("\"confidence\":0.75"));
        assert!(json.contains("\"score\":0.82"));
        assert!(json.contains("\"situation\":\"GameFocused\""));
    }

    #[test]
    fn spike_event_defaults_switch_prev_fields_for_old_json() {
        let s = SpikeEvent {
            task: 1,
            class: crate::process_tree::TaskClass::Game,
            comm: "test".to_owned(),
            process_comm: "test".into(),
            ..Default::default()
        };

        let mut val = serde_json::to_value(&s).unwrap();
        if let serde_json::Value::Object(ref mut map) = val {
            map.remove("switch_prev_pid");
            map.remove("switch_prev_state");
            map.remove("switch_prev_state_label");
        } else {
            panic!("expected object");
        }

        let json = serde_json::to_string(&val).unwrap();
        let decoded: SpikeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.switch_prev_pid, 0);
        assert_eq!(decoded.switch_prev_state, 0);
        assert_eq!(decoded.switch_prev_state_label, "");
    }

    #[test]
    fn test_spike_event_buffer_truncation() {
        let mut buf = crate::recorder::SpikeEventBuffer::with_max_events(1);
        let event = SpikeEvent {
            elapsed_ms: Some(0),
            task: 1,
            active: true,
            class: TaskClass::Unknown,
            process_pid: Some(1),
            process_comm: "test".into(),
            comm: "test".to_owned(),
            latency_ns: 1000,
            ..Default::default()
        };

        assert_eq!(
            buf.push(event.clone()),
            crate::recorder::SpikePushResult::Stored
        );
        assert_eq!(buf.push(event), crate::recorder::SpikePushResult::Dropped);
        assert!(buf.truncated());
    }

    #[test]
    fn test_spike_event_serialization() {
        let event = SpikeEvent {
            elapsed_ms: Some(100),
            task: 123,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(123),
            process_comm: "game".into(),
            comm: "game".to_owned(),
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 2000,
            switch_ns: 3000,
            major_faults: 1,
            minor_faults: 2,
            scx_ops: Some("scx_lavd".to_owned()),
            scx_state: Some("enabled".to_owned()),
            scx_enable_seq: Some("1".to_owned()),
            cause_tags: vec!["cpu_pressure".to_string()],
            primary_cause: Some("cpu_pressure".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"scx_ops\":\"scx_lavd\""));
        assert!(json.contains("\"scx_state\":\"enabled\""));

        let decoded: SpikeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.scx_ops.as_deref(), Some("scx_lavd"));
        assert_eq!(decoded.scx_state.as_deref(), Some("enabled"));
    }

    #[test]
    fn test_spike_event_deserialization_compatibility() {
        let json = r#"{"elapsed_ms":100,"task":123,"active":true,"class":"Game","process_pid":123,"process_comm":"game","comm":"game","cpu":1,"wakeup_target_cpu":1,"prio":120,"latency_ns":1000000,"wakeup_ns":2000,"switch_ns":3000,"target_pending_wakeups":0,"major_faults":1,"minor_faults":2}"#;
        let decoded: SpikeEvent = serde_json::from_str(json).unwrap();
        assert!(decoded.scx_ops.is_none());
        assert!(decoded.scx_state.is_none());
    }

    #[test]
    fn block_io_record_correlation_basis_serializes_as_string() {
        let record = BlockIoRecord {
            elapsed_ms: 1,
            tid: 42,
            correlation_basis: Cow::Borrowed("dev+sector"),
            dev: 1,
            nr_sector: 8,
            sector: 123,
            duration_ns: 456,
            timestamp_ns: 789,
            rwbs: "R".to_string(),
        };

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["correlation_basis"], "dev+sector");
        assert!(value.get("correlation_basis").unwrap().is_string());
    }

    #[test]
    fn block_io_record_correlation_basis_deserializes_from_string() {
        let json = serde_json::json!({
            "elapsed_ms": 1,
            "tid": 42,
            "correlation_basis": "request-pointer",
            "dev": 1,
            "nr_sector": 8,
            "sector": 123,
            "duration_ns": 456,
            "timestamp_ns": 789,
            "rwbs": "R"
        });

        let record: BlockIoRecord = serde_json::from_value(json).unwrap();

        assert_eq!(record.correlation_basis.as_ref(), "request-pointer");
    }
}
