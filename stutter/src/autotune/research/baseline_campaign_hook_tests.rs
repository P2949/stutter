//! The production-composed synthetic campaign over SIX parent arms with the real `FileProfileHook` editing real
//! per-arm `UserSettings.json` files. The screens run over all six, the pilots, confirmations and holdouts over the
//! two arms the screens kept; each study applies and restores its profile over exactly its own arms, every native
//! result seals the digest of the settings its arm's file actually had, and the independent verification binds
//! each result to its own arm's recorded digest.

use std::os::unix::fs::PermissionsExt;

use super::{baseline_campaign::NativeHost, *};
use crate::autotune::research::{
    ResearchTool,
    child_study::{Profile, Stage, StudyResult},
    funnel_driver::{FunnelDriver, RuleRegistry, production_inputs},
    profile_applier::{ProfileSpec, StudySettingsFile, ray_tracing_off},
    profile_hook::FileProfileHook,
    study_executor::ProfileFailure,
};

const SETTINGS: &str = r#"{"version":1,"data":[{"group_name":"/graphics/raytracing","options":[
{"name":"RayTracing","type":"bool","value":true},
{"name":"RayTracedReflections","type":"bool","value":true},
{"name":"RayTracedSunShadows","type":"bool","value":true},
{"name":"RayTracedLocalShadows","type":"bool","value":true},
{"name":"RayTracedPathTracing","type":"bool","value":false}]}]}"#;

const SIX: [ProtonSource; 6] = [
    ProtonSource::ValveStable,
    ProtonSource::ValveExperimental,
    ProtonSource::CachyOs,
    ProtonSource::CachyOsV3,
    ProtonSource::Ge,
    ProtonSource::Em,
];

fn spec(profile: Profile) -> Result<Option<ProfileSpec>, ProfileFailure> {
    Ok((profile != Profile::Long).then(ray_tracing_off))
}

fn idle() -> bool {
    false
}

struct Six {
    _root: tempfile::TempDir,
    settings: tempfile::TempDir,
    results: tempfile::TempDir,
    driver: FunnelDriver,
    parent: ResearchIntentV1,
    campaign: File,
}

fn six_arm_campaign() -> Six {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let results = tempfile::tempdir().unwrap();
    std::fs::set_permissions(results.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let settings = tempfile::tempdir().unwrap();
    let settings_root = std::fs::canonicalize(settings.path()).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let mut parent = parent();
    parent.tools = SIX
        .iter()
        .enumerate()
        .map(|(index, source)| ResearchTool {
            source: *source,
            identity_sha256: format!("{:064x}", index + 1),
        })
        .collect();
    parent.max_evidence_bytes = 256 * 1024 * 1024;
    parent.deadline_unix_seconds = parent.issued_unix_seconds + 40_000;
    parent.max_active_seconds = 28_000;
    parent.max_attempts = 400;
    let uid = crate::syscall::geteuid();
    let (mut files, mut arms) = (Vec::new(), Vec::new());
    for source in SIX {
        let dir = settings_root.join(source.dir_name());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("UserSettings.json");
        std::fs::write(&path, SETTINGS).unwrap();
        files.push(StudySettingsFile::new(&settings_root, path, uid).unwrap());
        arms.push(source);
    }
    let hook = FileProfileHook::new(files, spec, idle).with_arm_map(arms, Vec::new());
    let rules: RuleRegistry = Default::default();
    let host = NativeHost {
        settings_root: Some(settings_root),
        results: results.path().to_path_buf(),
        intent: parent.clone(),
        current: None,
        launches: 0,
        cold_until_initialized: false,
        initialized: false,
        initializing: false,
        warms_prefix: true,
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
        Box::new(hook),
        attempts,
        |_| None,
        rules,
    )
    .unwrap();
    Six {
        _root: root,
        settings,
        results,
        driver,
        parent,
        campaign,
    }
}

#[test]
fn six_parent_arms_narrowed_to_two_run_with_the_real_hook_and_every_result_binds_to_its_own_arms_settings()
 {
    use crate::autotune::research::driver::Step;
    let mut run = six_arm_campaign();
    let original = SETTINGS.as_bytes().to_vec();
    for _ in 0..40_000 {
        match run.driver.tick(&mut || Ok(())).unwrap() {
            Step::Done => break,
            Step::Stopped(_) | Step::Halted(_) | Step::OwnershipUnresolved(_) => panic!("stopped"),
            _ => {}
        }
    }
    let dispositions = run.driver.dispositions();
    assert!(
        dispositions
            .iter()
            .all(|d| d.result == Some(StudyResult::Qualified)),
        "{dispositions:?}"
    );
    // Every study restored what it edited: each arm's file is byte for byte what it was.
    for source in SIX {
        let path = run
            .settings
            .path()
            .join(source.dir_name())
            .join("UserSettings.json");
        assert_eq!(std::fs::read(&path).unwrap(), original, "{source:?}");
    }
    // Each study recorded digests for exactly the arms it ran.
    let campaign_path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap();
    let mut counts = std::collections::BTreeMap::<usize, Vec<Stage>>::new();
    for entry in std::fs::read_dir(campaign_path.join("studies"))
        .unwrap()
        .flatten()
    {
        let intent: ResearchIntentV1 =
            serde_json::from_slice(&std::fs::read(entry.path().join("intent.json")).unwrap())
                .unwrap();
        let recorded = crate::autotune::research::init_receipt::read_settings_receipt(
            &std::fs::read(
                entry
                    .path()
                    .join(crate::autotune::research::init_receipt::SETTINGS_RECEIPT),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            recorded.len(),
            intent.tools.len(),
            "one digest per arm the study ran"
        );
        let study = dispositions
            .iter()
            .find(|d| {
                d.study_sha256
                    .starts_with(entry.file_name().to_str().unwrap())
            })
            .unwrap();
        counts
            .entry(intent.tools.len())
            .or_default()
            .push(study.stage);
    }
    assert_eq!(
        counts.keys().copied().collect::<Vec<_>>(),
        [2, 6],
        "{counts:?}"
    );
    // The screens and the pilots (over every viable arm) ran across all six; confirmations and holdouts across two.
    assert!(
        counts[&6]
            .iter()
            .all(|stage| matches!(stage, Stage::Screen | Stage::Pilot))
    );
    assert!(counts[&2].iter().all(|stage| matches!(
        stage,
        Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs
    )));
    assert!(counts[&2].len() >= 4, "{counts:?}");
    // The independent verification binds each result to its own arm's digest and finds no problem.
    let results = run.results.path().to_path_buf();
    let verified = crate::autotune::research::child_verify::verify_children(
        &run.campaign,
        crate::syscall::geteuid(),
        &run.parent,
        &move |source| results.join(source.dir_name()),
    )
    .unwrap();
    assert!(
        verified.children.iter().all(|c| c.problem.is_none()),
        "{:?}",
        verified.children
    );
    assert!(verified.all_verified());
    // Scramble one two-arm study's record so that its second arm holds the first arm's digest: the digests differ per
    // arm (each arm's file is its own), so the result of the second arm no longer matches.
    let two = std::fs::read_dir(campaign_path.join("studies"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let intent: ResearchIntentV1 =
                serde_json::from_slice(&std::fs::read(p.join("intent.json")).unwrap()).unwrap();
            intent.tools.len() == 2
        })
        .unwrap();
    let file = two.join(crate::autotune::research::init_receipt::SETTINGS_RECEIPT);
    let mut digests = crate::autotune::research::init_receipt::read_settings_receipt(
        &std::fs::read(&file).unwrap(),
    )
    .unwrap();
    digests[1] = "f".repeat(64);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(
        &file,
        crate::autotune::research::init_receipt::settings_receipt_bytes(&digests).unwrap(),
    )
    .unwrap();
    let results = run.results.path().to_path_buf();
    let tampered = crate::autotune::research::child_verify::verify_children(
        &run.campaign,
        crate::syscall::geteuid(),
        &run.parent,
        &move |source| results.join(source.dir_name()),
    )
    .unwrap();
    assert!(tampered.children.iter().any(|c| c.problem.is_some()));
}
