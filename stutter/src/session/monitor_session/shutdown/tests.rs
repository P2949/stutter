//! Shutdown ordering tests for MonitorSession finalize.
//!
//! Verifies that the shutdown sequence is:
//!   1. Stop event ingestion (bus dropped)
//!   2. Flush recorder/exporters (streams finished)
//!   3. Detach probes (exporters, then ebpf dropped)
//!   4. Final report

use std::sync::{Arc, Mutex};

use super::{RecorderFlush, execute_shutdown_sequence};

struct DropTracker {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(self.name);
    }
}

/// Fake event bus whose drop signals "stop event ingestion".
struct FakeBus {
    _tracker: DropTracker,
}

/// Fake exporters whose drop signals "drop exporters".
struct FakeExporters {
    _tracker: DropTracker,
}

/// Fake eBPF handle whose drop signals "detach probes".
struct FakeEbpf {
    _tracker: DropTracker,
}

struct FakeRecorder {
    flush_log: Arc<Mutex<Vec<&'static str>>>,
}

impl RecorderFlush for FakeRecorder {
    fn flush_streams(&mut self) -> anyhow::Result<()> {
        self.flush_log
            .lock()
            .unwrap()
            .push("flush recorder/exporters");
        Ok(())
    }
}

#[test]
fn assert_shutdown_order() {
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let bus = FakeBus {
        _tracker: DropTracker {
            name: "stop event ingestion",
            log: log.clone(),
        },
    };
    let exporters = FakeExporters {
        _tracker: DropTracker {
            name: "drop exporters",
            log: log.clone(),
        },
    };
    let ebpf = FakeEbpf {
        _tracker: DropTracker {
            name: "detach probes",
            log: log.clone(),
        },
    };

    let mut recorder = FakeRecorder {
        flush_log: log.clone(),
    };

    let final_report_log = log.clone();
    let final_report = move |_recorder: &mut FakeRecorder| -> anyhow::Result<()> {
        final_report_log.lock().unwrap().push("final report");
        Ok(())
    };

    execute_shutdown_sequence(bus, &mut recorder, exporters, ebpf, final_report).unwrap();

    let actions = log.lock().unwrap().clone();
    assert_eq!(
        actions,
        vec![
            "stop event ingestion",
            "flush recorder/exporters",
            "drop exporters",
            "detach probes",
            "final report",
        ],
        "shutdown sequence must be: stop ingestion, flush, drop exporters, detach probes, final report"
    );
}
