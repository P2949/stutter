//! One production-composed synthetic campaign: the real funnel, journals, child studies, executor,
//! selection, raw-evidence replay and per-study reports, over a host that only injects the world. The
//! host writes real native result files (summary and frame file that agree) for every attempt and
//! returns a completion that seals their hashes with a matched runtime, so every integrity check the
//! production path makes runs against real files. Nothing here is measured on hardware.

use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use super::*;
use crate::autotune::research::{
    child_study::StudyResult,
    cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    },
    funnel_driver::{FunnelDriver, production_inputs},
    loaded_runtime::{LoadedRuntime, RuntimeBinding},
    study_executor::{ProfileFailure, ProfileHook},
};

/// The built-in benchmark is a fixed fly-through of this many seconds.
const SCENE_SECONDS: f64 = 64.25;

struct NoProfiles;
impl ProfileHook for NoProfiles {
    fn apply(
        &mut self,
        _: crate::autotune::research::child_study::Profile,
    ) -> Result<bool, ProfileFailure> {
        Ok(false)
    }
    fn recover(&mut self) -> Result<(), &'static str> {
        Ok(())
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        Ok(())
    }
}

/// Injects only the world: launching "runs" the game, polling returns a real result folder.
struct NativeHost {
    results: PathBuf,
    intent: ResearchIntentV1,
    current: Option<AttemptSlot>,
    launches: u32,
}

impl NativeHost {
    fn fps(slot: &AttemptSlot) -> f64 {
        let base = match slot.arm {
            ProtonSource::Ge => 80.0,
            _ => 70.0,
        };
        base + f64::from(slot.index % 3) * 0.1
    }

    fn write_result(&self, slot: &AttemptSlot) -> CyberpunkBenchmarkCompletion {
        let fps = Self::fps(slot);
        let frames = (fps * SCENE_SECONDS).round() as usize;
        let arm_dir = self.results.join(slot.arm.dir_name());
        let name = format!("run-{:04}", slot.index);
        let run = arm_dir.join(&name);
        std::fs::create_dir_all(&run).unwrap();
        for dir in [&arm_dir, &run] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let summary = format!(
            r#"{{"RootType":"worldBenchmarkSummary","Data":{{"averageFps":{fps},"frameNumber":{frames},"gameVersion":"2.5","presetName":"P","gpuName":"G","upscalingType":0,"rayTracingEnabled":false,"frameGenerationType":0}}}}"#
        );
        let mut csv = String::from("Frame index, Frame time (ms)\n");
        for index in 0..frames {
            csv.push_str(&format!("{index}, {:.6}\n", 1000.0 / fps));
        }
        std::fs::write(run.join("summary.json"), &summary).unwrap();
        std::fs::write(run.join("frames.csv"), &csv).unwrap();
        let context = crate::autotune::research::native_context::verify_frames(
            summary.as_bytes(),
            csv.as_bytes(),
        )
        .unwrap();
        let file = |file: &str, bytes: &[u8]| NativeBenchmarkFile {
            path: format!("{name}/{file}"),
            size_bytes: bytes.len() as u64,
            sha256: stutter_core::hash::sha256_hex(bytes),
        };
        let tool = self
            .intent
            .tools
            .iter()
            .find(|tool| tool.source == slot.arm)
            .unwrap();
        CyberpunkBenchmarkCompletion {
            schema_version: 1,
            app_id: self.intent.app_id,
            build_id: self.intent.build_id,
            scene: CyberpunkScenario::BuiltinBenchmark,
            settings_sha256: "c".repeat(64),
            proton_source: slot.arm,
            proton_identity_sha256: tool.identity_sha256.clone(),
            process: CyberpunkProcessIdentity {
                pid: stutter_core::ids::Pid::new(4000 + slot.index),
                start_ticks: 100 + u64::from(slot.index),
                uid: self.intent.uid,
                app_id: self.intent.app_id,
                build_id: self.intent.build_id,
                executable_sha256: self.intent.game_executable_sha256.clone(),
            },
            started_unix_nanos: 1,
            completed_unix_nanos: 2,
            result_directory: name.clone(),
            metrics: NativeBenchmarkMetrics {
                average_fps: fps,
                minimum_fps: None,
                maximum_fps: None,
            },
            context: Some(context),
            loaded_runtime: Some(LoadedRuntime {
                mapped_file_count: 3,
                wine_loaded: true,
                vkd3d_loaded: true,
                dxvk_loaded: true,
                proton_roots: vec!["/nix/store/aaa-proton/files".into()],
                runtime_paths_sha256: "a".repeat(64),
            }),
            runtime_binding: Some(RuntimeBinding::Matched),
            files: vec![
                file("frames.csv", csv.as_bytes()),
                file("summary.json", summary.as_bytes()),
            ],
        }
    }
}

impl AttemptHost for NativeHost {
    fn check_prefixes(
        &mut self,
        _: &[ProtonSource],
    ) -> Result<Option<crate::autotune::research::prefix_identity::PrefixClass>, &'static str> {
        // Every arm's prefix was initialized beforehand (the init phase, or a warm start).
        Ok(Some(
            crate::autotune::research::prefix_identity::PrefixClass::Warm,
        ))
    }
    fn attempt_index(&mut self) -> Option<u32> {
        self.current.as_ref().map(|slot| slot.index)
    }
    fn safe_window(&mut self) -> bool {
        true
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        Ok(ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        })
    }
    fn launch(&mut self, slot: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        self.launches += 1;
        self.current = Some(slot.clone());
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        let slot = self.current.clone().expect("polled before launch");
        let native = self.write_result(&slot);
        Progress::Finished {
            evidence: AttemptEvidenceV1 {
                schema_version: 1,
                slot_index: slot.index,
                average_fps: native.metrics.average_fps,
                native_completion: Some(Box::new(native)),
            },
        }
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn composed() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    FunnelDriver,
    ResearchIntentV1,
    File,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let results = tempfile::tempdir().unwrap();
    std::fs::set_permissions(results.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let mut parent = parent();
    parent.max_evidence_bytes = 256 * 1024 * 1024;
    parent.deadline_unix_seconds = parent.issued_unix_seconds + 40_000;
    parent.max_active_seconds = 28_000;
    parent.max_attempts = 400;
    let rules: crate::autotune::research::funnel_driver::RuleRegistry = Default::default();
    let host = NativeHost {
        results: results.path().to_path_buf(),
        intent: parent.clone(),
        current: None,
        launches: 0,
    };
    let results_root = results.path().to_path_buf();
    let mut now = parent.issued_unix_seconds + 1;
    let attempts = DriverStudyAttempts::new(
        campaign.try_clone().unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(move |source| results_root.join(source.dir_name())),
        production_inputs(campaign.try_clone().unwrap(), parent.clone(), rules.clone()),
    );
    let driver = FunnelDriver::new(
        &campaign,
        &parent,
        Box::new(NoProfiles),
        attempts,
        |_| None,
        rules,
    )
    .unwrap();
    (root, results, driver, parent, campaign)
}

struct Finished {
    _root: tempfile::TempDir,
    results: tempfile::TempDir,
    driver: FunnelDriver,
    parent: ResearchIntentV1,
    campaign: File,
}

/// Run the composed funnel until every study has a recorded decision.
fn run_campaign() -> Finished {
    let (root, results, mut driver, parent, campaign) = composed();
    for _ in 0..4000 {
        match driver.tick(&mut || Ok(())).unwrap() {
            Step::Done => {
                return Finished {
                    _root: root,
                    results,
                    driver,
                    parent,
                    campaign,
                };
            }
            stopped @ (Step::Stopped(_) | Step::Halted(_) | Step::OwnershipUnresolved(_)) => {
                panic!(
                    "the campaign stopped early: {stopped:?} after {:?}",
                    driver.dispositions()
                )
            }
            _ => {}
        }
    }
    panic!("the campaign never finished");
}

fn verify(run: &Finished) -> crate::autotune::research::child_verify::ChildrenVerification {
    let results_root = run.results.path().to_path_buf();
    crate::autotune::research::child_verify::verify_children(
        &run.campaign,
        crate::syscall::geteuid(),
        &run.parent,
        &move |source| results_root.join(source.dir_name()),
    )
    .unwrap()
}

#[test]
fn the_production_funnel_qualifies_screens_pilots_confirmations_and_holdouts_over_real_native_files()
 {
    let run = run_campaign();
    let all = run.driver.dispositions();
    assert_eq!(
        all.len(),
        8,
        "two profiles across screen, pilot, confirmation and holdout"
    );
    assert!(
        all.iter().all(|d| d.result == Some(StudyResult::Qualified)),
        "every baseline study qualifies over real native files: {all:?}"
    );
    // Independent verification: every child rebuilds from its own sealed journal, evidence and raw
    // native files, with the runtime matched and no problem, and each holdout says whether it replicated.
    let verified = verify(&run);
    assert!(verified.missing.is_empty());
    assert_eq!(verified.children.len(), 8);
    for child in &verified.children {
        assert_eq!(child.problem, None, "{}", child.study);
        assert!(child.reconstruction && child.raw_files_verified && child.runtime_matched);
        // Every study stood on the 99th-percentile frame time: the real frame files have a known population,
        // and the confirmation and holdout kept the metric their pilot froze.
        let report = child
            .report
            .as_ref()
            .expect("a verified child has its report");
        assert_eq!(
            report["primary_metric"], "native_p99_frame_ms",
            "{}",
            child.study
        );
    }
    assert_eq!(
        verified
            .children
            .iter()
            .filter(|c| c.replication.is_some())
            .count(),
        2,
        "each holdout names whether it replicated its confirmation"
    );
}

#[test]
fn a_tampered_native_file_is_found_by_independent_verification_of_the_finished_campaign() {
    let run = run_campaign();
    // Change one byte of one attempt's frame file after everything was sealed.
    let victim = std::fs::read_dir(run.results.path().join("ge"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("frames.csv");
    let mut bytes = std::fs::read(&victim).unwrap();
    bytes[40] ^= 1;
    std::fs::write(&victim, bytes).unwrap();
    let verified = verify(&run);
    assert!(
        verified
            .children
            .iter()
            .any(|child| child.problem.is_some() || !child.raw_files_verified),
        "a tampered result must not verify"
    );
}
