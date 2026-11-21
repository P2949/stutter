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

/// No settings file is touched; when `digests` is non-empty the profile claims to have applied and to have
/// left the settings files with those digests.
struct NoProfiles {
    digests: Vec<String>,
}
impl ProfileHook for NoProfiles {
    fn apply(
        &mut self,
        _: crate::autotune::research::child_study::Profile,
    ) -> Result<bool, ProfileFailure> {
        Ok(!self.digests.is_empty())
    }
    fn applied_digests(&self) -> Vec<String> {
        self.digests.clone()
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
    /// Arms whose prefix is cold until an initialization attempt has run (empty: all warm from the start).
    cold_until_initialized: bool,
    initialized: bool,
    initializing: bool,
    /// Whether an initialization launch actually leaves a warm prefix behind.
    warms_prefix: bool,
    initialization_launches: u32,
    screens_before_init: u32,
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
        use crate::autotune::research::prefix_identity::PrefixClass::{Cold, Warm};
        Ok(Some(if self.cold_until_initialized && !self.initialized {
            Cold
        } else {
            Warm
        }))
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
    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        self.launches += 1;
        self.initializing = context.intent.initialization;
        if self.initializing {
            self.initialization_launches += 1;
        } else if self.cold_until_initialized && !self.initialized {
            self.screens_before_init += 1;
        }
        self.current = Some(slot.clone());
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        let slot = self.current.clone().expect("polled before launch");
        if self.initializing {
            // The game created its prefix and was stopped: nothing was measured.
            self.initialized = self.warms_prefix;
            return Progress::Finished {
                evidence: AttemptEvidenceV1 {
                    thermal: None,
                    long_session: None,
                    schema_version: 1,
                    slot_index: slot.index,
                    average_fps: 1.0,
                    native_completion: None,
                },
            };
        }
        let native = self.write_result(&slot);
        Progress::Finished {
            evidence: AttemptEvidenceV1 {
                thermal: None,
                long_session: None,
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

fn composed(
    init_arms: Vec<ProtonSource>,
    warms_prefix: bool,
    profile_digests: Vec<String>,
) -> (
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
    parent.prefix_init_arms = init_arms;
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
        cold_until_initialized: !parent.prefix_init_arms.is_empty(),
        initialized: false,
        initializing: false,
        warms_prefix,
        initialization_launches: 0,
        screens_before_init: 0,
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
        Box::new(NoProfiles {
            digests: profile_digests,
        }),
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
    run_campaign_with(Vec::new(), true)
}

fn run_campaign_with_profile(digests: Vec<String>) -> Finished {
    run_campaign_full(Vec::new(), true, digests)
}

fn run_campaign_with(init_arms: Vec<ProtonSource>, warms_prefix: bool) -> Finished {
    run_campaign_full(init_arms, warms_prefix, Vec::new())
}

fn run_campaign_full(
    init_arms: Vec<ProtonSource>,
    warms_prefix: bool,
    profile_digests: Vec<String>,
) -> Finished {
    let (root, results, mut driver, parent, campaign) =
        composed(init_arms, warms_prefix, profile_digests);
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
fn the_campaigns_own_raw_copy_is_what_is_verified_and_tampering_with_it_is_found() {
    let run = run_campaign();
    // The prefix's results directory is not the campaign's to control: once the sealed files are
    // archived, rotating or editing it changes nothing about verification.
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
    std::fs::remove_dir_all(run.results.path().join("valve_stable")).unwrap();
    let verified = verify(&run);
    assert!(
        verified
            .children
            .iter()
            .all(|child| child.problem.is_none() && child.raw_files_verified),
        "verification stands on the archived copy"
    );

    // Editing the archived copy is found.
    let campaign_path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap();
    let archived = std::fs::read_dir(campaign_path.join("studies"))
        .unwrap()
        .flatten()
        .map(|study| study.path().join("raw/ge"))
        .find(|path| path.is_dir())
        .unwrap();
    let frames = std::fs::read_dir(&archived)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("frames.csv");
    let mut bytes = std::fs::read(&frames).unwrap();
    bytes[40] ^= 1;
    std::fs::set_permissions(&frames, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&frames, bytes).unwrap();
    let verified = verify(&run);
    assert!(
        verified
            .children
            .iter()
            .any(|child| child.problem.is_some() || !child.raw_files_verified),
        "a tampered archive copy must not verify"
    );
}

#[test]
fn the_cumulative_budgets_replay_from_what_the_children_sealed() {
    let run = run_campaign();
    let clean = verify(&run);
    assert_eq!(clean.budget_problem, None);
    assert!(clean.all_verified());

    let campaign_path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap();
    let studies = campaign_path.join("studies");
    let first = std::fs::read_dir(&studies)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let record = first.join("budget.json");
    let original = std::fs::read(&record).unwrap();
    let rewrite = |bytes: &[u8]| {
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&record, bytes).unwrap();
    };

    // A spend record that understates the sealed evidence is found.
    let mut spent: crate::autotune::research::campaign_budget::Spent =
        serde_json::from_slice(&original).unwrap();
    spent.evidence_bytes -= 1;
    rewrite(&serde_json::to_vec(&spent).unwrap());
    let problem = verify(&run).budget_problem.unwrap();
    assert!(
        problem.starts_with("BudgetEvidenceBytesMismatch"),
        "{problem}"
    );
    assert!(!verify(&run).all_verified());

    // A total over the authorized active time is found.
    spent = serde_json::from_slice(&original).unwrap();
    spent.active_seconds = u64::from(run.parent.max_active_seconds) + 1;
    rewrite(&serde_json::to_vec(&spent).unwrap());
    assert_eq!(
        verify(&run).budget_problem.as_deref(),
        Some("CampaignActiveSecondsExceeded")
    );

    // A child with no spend record is found rather than assumed free.
    std::fs::remove_file(&record).unwrap();
    let problem = verify(&run).budget_problem.unwrap();
    assert!(problem.starts_with("BudgetRecordMissing"), "{problem}");
}

#[test]
fn a_cold_arm_is_initialized_by_its_own_study_before_any_screen_and_the_campaign_then_verifies() {
    let run = run_campaign_with(vec![ProtonSource::Ge], true);
    let all = run.driver.dispositions();
    assert_eq!(
        all.len(),
        9,
        "the initialization study joins the baseline eight"
    );
    assert!(
        all.iter().all(|d| d.result == Some(StudyResult::Qualified)),
        "{all:?}"
    );
    // It runs first, and nothing measured ran on the cold prefix.
    assert_eq!(
        all[0].stage,
        crate::autotune::research::child_study::Stage::Init
    );
    let verified = verify(&run);
    assert_eq!(verified.children.len(), 9);
    assert!(verified.missing.is_empty() && verified.budget_problem.is_none());
    for child in &verified.children {
        assert_eq!(child.problem, None, "{}", child.study);
    }
    assert!(verified.all_verified());
}

#[test]
fn an_initialization_that_leaves_the_prefix_cold_blocks_every_screen_rather_than_comparing_cold_prefixes()
 {
    use crate::autotune::research::child_study::Stage;
    let run = run_campaign_with(vec![ProtonSource::Ge], false);
    let all = run.driver.dispositions();
    let init = &all[0];
    assert_eq!(init.stage, Stage::Init);
    assert_eq!(init.result, Some(StudyResult::Unqualified));
    assert_eq!(init.reason, "InitializationUnproven");
    // Nothing that depends on it ran: every other study is blocked by its predecessor.
    assert!(
        all[1..].iter().all(
            |d| d.result == Some(StudyResult::Blocked) && d.reason == "PredecessorNotQualified"
        ),
        "{all:?}"
    );
}

#[test]
fn every_result_must_have_run_with_the_settings_the_applied_profile_left() {
    use crate::autotune::research::child_study::Stage;
    // The synthetic results seal this settings hash; a profile that left exactly it conforms.
    let ran_with = "c".repeat(64);
    let run = run_campaign_with_profile(vec!["d".repeat(64), ran_with.clone()]);
    let all = run.driver.dispositions();
    assert!(
        all.iter().all(|d| d.result == Some(StudyResult::Qualified)),
        "{all:?}"
    );
    let verified = verify(&run);
    assert!(verified.all_verified());
    assert!(verified.children.iter().all(|c| c.problem.is_none()));

    // A recorded digest list that does not contain what a result ran with is found offline: the study
    // can no longer be the qualified one it claims to be.
    let campaign_path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap();
    let screen = std::fs::read_dir(campaign_path.join("studies"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.join(crate::autotune::research::init_receipt::SETTINGS_RECEIPT)
                .is_file()
        })
        .expect("a study recorded the settings its profile left");
    let receipt = screen.join(crate::autotune::research::init_receipt::SETTINGS_RECEIPT);
    std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(
        &receipt,
        crate::autotune::research::init_receipt::settings_receipt_bytes(&["e".repeat(64)]).unwrap(),
    )
    .unwrap();
    let tampered = verify(&run);
    assert!(
        tampered.children.iter().any(|c| c.problem.is_some()),
        "a result that ran with other settings must not verify as qualified"
    );
    let _ = Stage::Screen;
}

#[test]
fn a_profile_that_left_other_settings_than_the_game_ran_with_is_unqualified() {
    use crate::autotune::research::child_study::Stage;
    let run = run_campaign_with_profile(vec!["e".repeat(64)]);
    let all = run.driver.dispositions();
    let screen = all.iter().find(|d| d.stage == Stage::Screen).unwrap();
    assert_eq!(screen.result, Some(StudyResult::Unqualified), "{all:?}");
    assert_eq!(screen.reason, "ProfileViolated");
}
