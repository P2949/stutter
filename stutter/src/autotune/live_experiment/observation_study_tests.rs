use std::{
    io::Write,
    os::unix::{fs::DirBuilderExt, net::UnixStream},
    path::{Path, PathBuf},
};

use super::*;
use crate::autotune::{
    experiment_design::{Decision, StoppingRule},
    objective::source::{KpiBlockRequest, KpiCollector, SourceBinding},
};

struct Rig {
    collector: KpiCollector,
    stream: UnixStream,
    dir: PathBuf,
    contract: KpiContract,
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn now() -> u64 {
    crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap()
}

impl Rig {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("stutter-kpi-study-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let pid = std::process::id();
        let live = crate::telemetry::proc_view::live_process(Path::new("/proc"), pid).unwrap();
        let contract = crate::autotune::objective::source::audio_xrun(SourceBinding {
            pid,
            uid: live.uid,
            start_ticks: live.start_ticks,
        })
        .contract;
        let collector = KpiCollector::bind(contract.clone(), &dir).unwrap();
        let stream = UnixStream::connect(dir.join("kpi.sock")).unwrap();
        Self {
            collector,
            stream,
            dir,
            contract,
        }
    }
    fn config(&self) -> KpiObservationStudyConfig {
        KpiObservationStudyConfig {
            design: ExperimentDesign {
                schema_version: 1,
                objective: "experimental test fixture".into(),
                metric: self.contract.primary.id.clone(),
                unit: self.contract.primary.unit.clone(),
                candidate: "observation comparison only".into(),
                context_fingerprint: "ctx".into(),
                min_effect_percent: 20.0,
                equivalence_margin_percent: 5.0,
                seed: 42,
                block_ids: (0..4).map(|i| format!("block-{i}")).collect(),
                stopping: StoppingRule::FixedN {
                    blocks: 4,
                    confidence: 0.95,
                },
                warmup_seconds: 0,
                washout_seconds: 0,
            },
            contract: self.contract.clone(),
            work_unit: "fixed-input".into(),
            hard_guardrail_regression_percent: BTreeMap::from([("xrun_count".into(), 0.0)]),
        }
    }
    fn capture(
        &mut self,
        pair: u32,
        arm: &str,
        primary: f64,
        guard: f64,
        kind: &str,
    ) -> anyhow::Result<ObservedKpiBlock> {
        let start = now();
        let expected;
        {
            let binding = self.contract.binding;
            let report = self.collector.poll(
                Some((binding.pid, binding.start_ticks)),
                Path::new("/proc"),
                start,
                1_000_000_000,
            );
            expected = report.accepted_samples + 2;
        }
        for (metric, value) in [("deadline_miss_count", primary), ("xrun_count", guard)] {
            writeln!(
                self.stream,
                "{{\"metric\":\"{metric}\",\"value\":{value},\"monotonic_ns\":{start},\"work_unit\":\"fixed-input\",\"population\":1000,\"kind\":\"{kind}\"}}"
            )?;
        }
        for _ in 0..30 {
            let binding = self.contract.binding;
            let report = self.collector.poll(
                Some((binding.pid, binding.start_ticks)),
                Path::new("/proc"),
                now(),
                1_000_000_000,
            );
            if report.accepted_samples == expected {
                break;
            }
        }
        self.collector.capture_block(KpiBlockRequest {
            block_id: format!("block-{pair}"),
            run_id: format!("run-{pair}-{arm}"),
            work_unit: "fixed-input".into(),
            context_fingerprint: "ctx".into(),
            from_ns: start,
            to_ns: now(),
        })
    }
    fn pair(
        &mut self,
        index: u32,
        treatment: f64,
        guard: f64,
    ) -> (ObservedKpiBlock, ObservedKpiBlock) {
        if index.is_multiple_of(2) {
            let baseline = self
                .capture(index, "baseline", 100.0, 100.0, "batch_sum")
                .unwrap();
            let treatment = self
                .capture(index, "treatment", treatment, guard, "batch_sum")
                .unwrap();
            (baseline, treatment)
        } else {
            let treatment = self
                .capture(index, "treatment", treatment, guard, "batch_sum")
                .unwrap();
            let baseline = self
                .capture(index, "baseline", 100.0, 100.0, "batch_sum")
                .unwrap();
            (baseline, treatment)
        }
    }
}

#[test]
fn actual_collector_blocks_reconstruct_beneficial_neutral_harmful_and_guard_failure_without_actions()
 {
    for (name, treatment, guard, decision) in [
        ("good", 70.0, 100.0, Decision::Beneficial),
        ("neutral", 100.0, 100.0, Decision::Equivalent),
        ("harm", 130.0, 100.0, Decision::Harmful),
        ("guard", 70.0, 101.0, Decision::InvalidExperiment),
    ] {
        let mut rig = Rig::new(name);
        let config = rig.config();
        let mut manager = LiveExperimentManager::new();
        manager.begin_kpi_observation_study(config.clone()).unwrap();
        assert!(
            manager.analyse_kpi_observation_study(1).is_err(),
            "fixed N refuses an early look"
        );
        let mut pairs = Vec::new();
        for index in 0..4 {
            let pair = rig.pair(index, treatment, guard);
            manager.observe_kpi_pair(&pair.0, &pair.1).unwrap();
            pairs.push(pair);
        }
        let evidence = manager.analyse_kpi_observation_study(1).unwrap();
        assert_eq!(evidence.evidence.decision, decision);
        assert!(!evidence.authorizes_mutation);
        assert!(!manager.has_active_experiment());
        assert!(manager.pending_objective_evidence.is_none());
        let refs: Vec<_> = pairs.iter().map(|(a, b)| (a, b)).collect();
        evidence.verify_captures(&config, &refs).unwrap();
        let mut forged = evidence.clone();
        forged.evidence.analyses[0] = config.design.analyse(&[50.0; 4], 1, false).unwrap();
        forged.evidence.decision = Decision::Beneficial;
        assert!(
            forged.evidence.consistent_with(&config.design),
            "a fabricated interval can be internally coherent"
        );
        assert!(
            forged.verify_captures(&config, &refs).is_err(),
            "raw reconstruction rejects that false keep claim"
        );
        assert!(
            manager.analyse_kpi_observation_study(1).is_err(),
            "analysis look is single-use"
        );
    }
}

#[test]
fn predesign_replayed_missing_summary_and_drifted_blocks_cannot_supply_valid_evidence() {
    let mut rig = Rig::new("denials");
    assert!(rig.capture(0, "summary", 100.0, 100.0, "summary").is_err());
    let before = rig.pair(0, 70.0, 100.0);
    let mut manager = LiveExperimentManager::new();
    let config = rig.config();
    manager.begin_kpi_observation_study(config.clone()).unwrap();
    assert!(
        manager.observe_kpi_pair(&before.0, &before.1).is_err(),
        "no retrospective design"
    );
    // A fresh manager is an independent fixture, not a retry of a real performance study.
    let mut manager = LiveExperimentManager::new();
    manager.begin_kpi_observation_study(config.clone()).unwrap();
    let pair = rig.pair(0, 70.0, 100.0);
    manager.observe_kpi_pair(&pair.0, &pair.1).unwrap();
    assert!(manager.observe_kpi_pair(&pair.0, &pair.1).is_err());
    let mut wrong = config;
    wrong.design.context_fingerprint = "different-context".into();
    let mut manager = LiveExperimentManager::new();
    manager.begin_kpi_observation_study(wrong).unwrap();
    let pair = rig.pair(0, 70.0, 100.0);
    assert!(manager.observe_kpi_pair(&pair.0, &pair.1).is_err());
}

#[test]
fn losing_and_reacquiring_the_same_target_cannot_transfer_the_old_study_session() {
    let mut rig = Rig::new("focus-epoch");
    let mut manager = LiveExperimentManager::new();
    manager.begin_kpi_observation_study(rig.config()).unwrap();
    let pair = rig.pair(0, 70.0, 100.0);
    manager.observe_kpi_pair(&pair.0, &pair.1).unwrap();
    rig.collector.poll(None, Path::new("/proc"), now(), 1);
    rig.stream = UnixStream::connect(rig.dir.join("kpi.sock")).unwrap();
    let pair = rig.pair(1, 70.0, 100.0);
    assert!(
        manager
            .observe_kpi_pair(&pair.0, &pair.1)
            .unwrap_err()
            .to_string()
            .contains("source session changed")
    );
}
