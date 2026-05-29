//! Tests for diagnosis scoring, evidence, candidate ranking, and anchor selection.
//!
//! Owns diagnosis regression tests and test-only fixtures. Does not own production diagnosis
//! configuration, models, evidence builders, or orchestration.

use std::collections::BTreeSet;

use super::*;
use crate::{
    process_tree::TaskClass,
    recorder::{BlockIoRecord, GpuSample, IntervalRecord, IrqEventRecord},
    session_io::RunArtifacts,
    spike::{SpikeCluster, SpikePoint},
};

fn spike_point(task: u32, class: TaskClass, comm: &str, latency_ns: u64) -> SpikePoint {
    let switch_ns = 100_000_000 + u64::from(task);
    SpikePoint {
        task,
        class,
        process_pid: Some(task),
        comm: comm.to_owned(),
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        elapsed_ms: Some(100),
        ..Default::default()
    }
}

fn spike_cluster(points: Vec<SpikePoint>) -> SpikeCluster {
    let distinct_tasks = points
        .iter()
        .map(|point| point.task)
        .collect::<BTreeSet<_>>()
        .len();
    let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
    let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
    let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        ..Default::default()
    }
}

fn irq_event(duration_ns: u64) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: Some(100),
        irq: 137,
        cpu: 0,
        enter_ns: 100_000_000,
        exit_ns: 100_000_000 + duration_ns,
        duration_ns,
    }
}

fn block_io_event(duration_ns: u64) -> BlockIoRecord {
    BlockIoRecord {
        elapsed_ms: 100,
        tid: 100.into(),
        dev: 1,
        nr_sector: 8,
        sector: 2048,
        duration_ns,
        timestamp_ns: 100_500_000,
        rwbs: "R".to_owned(),
        ..Default::default()
    }
}

fn cpu_psi_interval(cpu_psi_some: f64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms: 100,
        task: 100,
        active: true,
        class: TaskClass::Unknown,
        comm: "worker-a".to_owned(),
        process_pid: Some(100),
        process_comm: "worker-a".into(),
        samples: 1,
        stored_samples: 1,
        cpu_psi_some,
        ..Default::default()
    }
}

fn candidate(diagnosis: &Diagnosis, cause: StutterCause) -> &DiagnosisCandidate {
    diagnosis
        .candidates
        .iter()
        .find(|candidate| candidate.cause == cause)
        .unwrap()
}

fn candidate_index(diagnosis: &Diagnosis, cause: StutterCause) -> Option<usize> {
    diagnosis
        .candidates
        .iter()
        .position(|candidate| candidate.cause == cause)
}

fn assert_no_candidate(diagnosis: &Diagnosis, cause: StutterCause) {
    assert!(
        !diagnosis
            .candidates
            .iter()
            .any(|candidate| candidate.cause == cause),
        "unexpected candidate {:?}: {:#?}",
        cause,
        diagnosis
    );
}

fn assert_missing_contains(diagnosis: &Diagnosis, needle: &str) {
    assert!(
        diagnosis
            .missing_evidence
            .iter()
            .any(|message| message.contains(needle)),
        "missing_evidence did not contain {:?}: {:?}",
        needle,
        diagnosis.missing_evidence
    );
}

mod confidence;

mod evidence;

mod candidates;

mod orchestration;

mod anchor;
