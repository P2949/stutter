//! The production child-study funnel, selected only when the administrator enables it. Both the
//! funnel switch and live launch must be on and the study directory configured; anything else
//! installs no builder, so every campaign keeps running the flat schedule exactly as before.

use std::fs::File;

use super::FunnelBuilder;
use crate::{
    autotune::research::{
        ProtonSource, ResearchIntentV1,
        action_gate::{ActionGate, SharedActionGate},
        driver::AttemptHost,
        driver_attempts::DriverStudyAttempts,
        funnel_driver::{FunnelDriver, RuleRegistry, production_inputs, selection_blocker},
        profile_applier::StudySettingsFile,
        profile_hook::{FileProfileHook, cyberpunk_running, production_spec},
        session_host::arm_prefix_paths,
        study_executor::{ProfileFailure, ProfileHook},
        telemetry_observer::SharedTelemetry,
    },
    daemon::privilege::UnixSocketPrivilegedActionService,
};

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The remembered list mappings plus what the arms' own settings files show now, saved back as an
/// immutable observation. Untrusted remembered mappings (unreadable or contradictory) are not used,
/// and an arm whose observation contradicts what is known is ignored, never allowed to overwrite it.
fn observed_mappings(
    study_dir: &std::path::Path,
    uid: u32,
    files: &[StudySettingsFile],
) -> Result<crate::autotune::research::profile_mappings::Mappings, ProfileFailure> {
    use crate::autotune::research::profile_mappings::Mappings;
    let dir =
        File::open(study_dir).map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
    let mut known =
        Mappings::load(&dir, uid).map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
    let mut fresh = Mappings::default();
    for file in files {
        let bytes = file
            .read()
            .map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
        let seen =
            Mappings::observe(&bytes).ok_or(ProfileFailure::Blocked("BlockedProfileMapping"))?;
        known
            .merge(&seen)
            .map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
        fresh
            .merge(&seen)
            .map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
    }
    fresh
        .save(&dir)
        .map_err(|_| ProfileFailure::Blocked("BlockedProfileMapping"))?;
    Ok(known)
}

/// The graphics-profile hook over every arm's own `UserSettings.json`. If any of them cannot be
/// opened safely (a cold prefix has none yet), the hook says so instead of editing a subset.
fn profiles(study_dir: &std::path::Path, intent: &ResearchIntentV1) -> Box<dyn ProfileHook + Send> {
    // Each arm is opened on its own: the ones that can be opened are recovered and restored whatever
    // the others are, and the ones that cannot (no file in a cold prefix, an unsafe one) only refuse
    // a profile, never hide a snapshot beside them.
    let (mut files, mut unavailable) = (Vec::new(), Vec::new());
    for tool in &intent.tools {
        let settings = arm_prefix_paths(study_dir, tool.source).2;
        match StudySettingsFile::new(study_dir, settings.clone(), intent.uid) {
            Ok(file) => files.push(file),
            Err(_) => unavailable.push(settings),
        }
    }
    // The list mappings observed so far (remembered beside the study) and any the arms' current
    // settings now show. A profile is resolved against them; a value never observed keeps it blocked.
    let mapping_dir = study_dir.to_path_buf();
    let uid = intent.uid;
    Box::new(
        FileProfileHook::new(files, production_spec, cyberpunk_running)
            .with_unavailable(unavailable)
            .with_mapping_loader(Box::new(move |files| {
                observed_mappings(&mapping_dir, uid, files)
            })),
    )
}

/// Rediscover prefix settings and their observed mappings immediately before each profile.
/// The active hook is retained until restoration is proven; replacing it earlier would lose
/// in-memory rollback state. Both the old and freshly discovered settings are recovered before use.
struct RefreshingProfiles {
    make: Box<dyn Fn() -> Box<dyn ProfileHook + Send> + Send>,
    current: Box<dyn ProfileHook + Send>,
    active: bool,
    unresolved: bool,
}

impl RefreshingProfiles {
    fn new(make: Box<dyn Fn() -> Box<dyn ProfileHook + Send> + Send>) -> Self {
        let current = make();
        Self {
            make,
            current,
            active: false,
            unresolved: false,
        }
    }

    fn recover_current(&mut self) -> Result<(), &'static str> {
        if self.active || self.unresolved {
            return Err("ProfileOwnershipUnresolved");
        }
        if let Err(reason) = self.current.recover() {
            self.unresolved = true;
            return Err(reason);
        }
        Ok(())
    }
}

impl ProfileHook for RefreshingProfiles {
    fn apply(
        &mut self,
        profile: crate::autotune::research::child_study::Profile,
    ) -> Result<bool, ProfileFailure> {
        self.recover_current().map_err(ProfileFailure::Unrestored)?;
        self.current = (self.make)();
        self.recover_current().map_err(ProfileFailure::Unrestored)?;
        let result = self.current.apply(profile);
        self.active = matches!(result, Ok(true));
        self.unresolved = matches!(result, Err(ProfileFailure::Unrestored(_)));
        result
    }

    fn recover(&mut self) -> Result<(), &'static str> {
        self.recover_current()
    }

    fn applied_digests(&self) -> Vec<String> {
        self.current.applied_digests()
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        if self.unresolved {
            return Err("ProfileOwnershipUnresolved");
        }
        if let Err(reason) = self.current.restore() {
            self.unresolved = true;
            return Err(reason);
        }
        self.active = false;
        Ok(())
    }
}

fn build(
    study_dir: &std::path::Path,
    worker_socket: Option<&std::path::Path>,
    intent: &ResearchIntentV1,
    campaign: &File,
    host: Box<dyn AttemptHost + Send>,
) -> anyhow::Result<FunnelDriver> {
    // The action child's gate: unarmed until an Action study arms it on the sighted game process.
    // Its restoration proof reads the worker's outstanding-token count; unknown stays unproven.
    let socket = worker_socket.map(std::path::Path::to_path_buf);
    let gate: SharedActionGate = std::sync::Arc::new(std::sync::Mutex::new(
        ActionGate::unarmed(Box::new(move || {
            let socket = socket.clone()?;
            UnixSocketPrivilegedActionService::new(socket)
                .outstanding_tokens()
                .ok()
                .and_then(|count| u32::try_from(count).ok())
        }))
        // A restoration is also compared with the target's own kernel state, read from /proc.
        .with_state_probe(Box::new(
            crate::autotune::research::action_gate::procfs_state,
        )),
    ));
    let telemetry: SharedTelemetry = Default::default();
    let rules = RuleRegistry::default();
    let results = study_dir.to_path_buf();
    // The driver rejects a clock that moves backward, so the clock is clamped to never do so.
    let mut last = 0u64;
    let mut attempts = DriverStudyAttempts::new(
        campaign.try_clone()?,
        intent.clone(),
        host,
        Box::new(move || {
            last = last.max(unix_now());
            last
        }),
        // The controller passes its own lease fence on every tick.
        Box::new(|| Ok(())),
        Box::new(move |arm: ProtonSource| arm_prefix_paths(&results, arm).1),
        production_inputs(campaign.try_clone()?, intent.clone(), rules.clone()),
    )
    .with_action_gate(gate.clone())
    .with_telemetry(telemetry.clone());
    if let Some(socket) = worker_socket.map(std::path::Path::to_path_buf) {
        attempts = attempts.with_claim_verifier_factory(Box::new(move || {
            super::drive::production_claim_verifier(socket.clone())
        }));
    }
    // A treatment study no measured build can run is recorded as blocked before it starts.
    let screened = selection_blocker(campaign.try_clone()?, intent.clone(), rules.clone());
    // A study on the long profile runs only when the intent froze an authorized work unit that fits its
    // window; otherwise it is recorded as blocked, never run on the native benchmark and reported as a
    // long session.
    let long_window = intent.long_attempt_seconds;
    let long_unit = intent.long_work_unit.clone();
    let selection: crate::autotune::research::funnel_driver::SelectionBlocker =
        Box::new(move |study| {
            crate::autotune::research::long_session::long_unit_blocker(
                study,
                long_unit.as_ref(),
                long_window,
            )
            .or_else(|| screened(study))
        });
    Ok(FunnelDriver::new(
        campaign,
        intent,
        Box::new(RefreshingProfiles::new(Box::new({
            let study_dir = study_dir.to_path_buf();
            let intent = intent.clone();
            move || profiles(&study_dir, &intent)
        }))),
        attempts,
        |_| None,
        rules,
    )?
    .with_selection_blocker(selection)
    .with_telemetry(telemetry)
    .with_action_gate(gate))
}

/// The funnel builder the resident session installs: `None` (the flat schedule) unless the
/// child-study funnel and live launch are both explicitly enabled and the study directory is set.
pub(in crate::autotune::runtime) fn select_funnel_builder(
    config: &crate::daemon::config::autotune::DaemonAutotuneConfig,
) -> Option<FunnelBuilder> {
    if !(config.research_child_funnel && config.research_live_launch) {
        return None;
    }
    let study_dir = config.research_study_dir.clone()?;
    let socket = config
        .privileged_worker_socket
        .clone()
        .or_else(|| crate::daemon::privilege::default_privileged_worker_socket_path().ok());
    Some(Box::new(move |intent, campaign, host| {
        build(&study_dir, socket.as_deref(), intent, campaign, host)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> crate::daemon::config::autotune::DaemonAutotuneConfig {
        crate::daemon::config::autotune::DaemonAutotuneConfig::default()
    }

    #[derive(Default)]
    struct RefreshState {
        settings_exist: bool,
        fail_recovery: bool,
        fail_restore: bool,
        generations: usize,
        applications: Vec<(usize, bool)>,
        restores: Vec<usize>,
    }

    struct FakeProfile {
        state: std::sync::Arc<std::sync::Mutex<RefreshState>>,
        generation: usize,
        settings_exist: bool,
    }

    impl ProfileHook for FakeProfile {
        fn apply(
            &mut self,
            _: crate::autotune::research::child_study::Profile,
        ) -> Result<bool, ProfileFailure> {
            let mut state = self.state.lock().unwrap();
            state
                .applications
                .push((self.generation, self.settings_exist));
            if self.settings_exist {
                Ok(true)
            } else {
                Err(ProfileFailure::Blocked("BlockedPrefixNotInitialized"))
            }
        }
        fn recover(&mut self) -> Result<(), &'static str> {
            if self.state.lock().unwrap().fail_recovery {
                Err("RecoveryUnproven")
            } else {
                Ok(())
            }
        }
        fn restore(&mut self) -> Result<(), &'static str> {
            let mut state = self.state.lock().unwrap();
            state.restores.push(self.generation);
            if state.fail_restore {
                Err("RestoreUnproven")
            } else {
                Ok(())
            }
        }
    }

    fn refreshing(state: std::sync::Arc<std::sync::Mutex<RefreshState>>) -> RefreshingProfiles {
        RefreshingProfiles::new(Box::new(move || {
            let mut locked = state.lock().unwrap();
            locked.generations += 1;
            Box::new(FakeProfile {
                state: state.clone(),
                generation: locked.generations,
                settings_exist: locked.settings_exist,
            })
        }))
    }

    #[test]
    fn profile_refresh_observes_new_settings_and_preserves_the_applied_hook_until_restore() {
        use crate::autotune::research::child_study::Profile;
        let state = std::sync::Arc::new(std::sync::Mutex::new(RefreshState::default()));
        let mut hook = refreshing(state.clone());
        hook.recover().unwrap();
        state.lock().unwrap().settings_exist = true;
        assert_eq!(hook.apply(Profile::Cpu), Ok(true));
        assert_eq!(state.lock().unwrap().applications, vec![(2, true)]);
        assert!(matches!(
            hook.apply(Profile::Cpu),
            Err(ProfileFailure::Unrestored(_))
        ));
        assert_eq!(state.lock().unwrap().generations, 2);
        hook.restore().unwrap();
        assert_eq!(state.lock().unwrap().restores, vec![2]);
        assert_eq!(hook.apply(Profile::Cpu), Ok(true));
        assert_eq!(state.lock().unwrap().generations, 3);
    }

    #[test]
    fn unproven_recovery_latches_the_profile_hook_and_prevents_rediscovery() {
        use crate::autotune::research::child_study::Profile;
        let state = std::sync::Arc::new(std::sync::Mutex::new(RefreshState::default()));
        let mut hook = refreshing(state.clone());
        state.lock().unwrap().fail_recovery = true;
        assert_eq!(
            hook.apply(Profile::Cpu),
            Err(ProfileFailure::Unrestored("RecoveryUnproven"))
        );
        state.lock().unwrap().fail_recovery = false;
        assert!(matches!(
            hook.apply(Profile::Cpu),
            Err(ProfileFailure::Unrestored(_))
        ));
        assert!(hook.restore().is_err());
        assert_eq!(state.lock().unwrap().generations, 1);
        assert!(state.lock().unwrap().applications.is_empty());
    }

    #[test]
    fn unproven_restoration_does_not_replace_or_reapply_the_hook() {
        use crate::autotune::research::child_study::Profile;
        let state = std::sync::Arc::new(std::sync::Mutex::new(RefreshState {
            settings_exist: true,
            ..Default::default()
        }));
        let mut hook = refreshing(state.clone());
        assert_eq!(hook.apply(Profile::Cpu), Ok(true));
        state.lock().unwrap().fail_restore = true;
        assert_eq!(hook.restore(), Err("RestoreUnproven"));
        state.lock().unwrap().fail_restore = false;
        assert!(matches!(
            hook.apply(Profile::Cpu),
            Err(ProfileFailure::Unrestored(_))
        ));
        assert!(hook.restore().is_err());
        assert_eq!(state.lock().unwrap().generations, 2);
    }

    #[test]
    fn profile_refresh_reopens_settings_created_after_funnel_construction() {
        use crate::autotune::research::{
            child_study::Profile,
            profile_applier::{ProfileSpec, ray_tracing_off},
        };
        fn spec(_: Profile) -> Result<Option<ProfileSpec>, ProfileFailure> {
            Ok(Some(ray_tracing_off()))
        }
        fn idle() -> bool {
            false
        }
        let root = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let settings = root_path.join("UserSettings.json");
        let observations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut hook = RefreshingProfiles::new(Box::new({
            let root_path = root_path.clone();
            let settings = settings.clone();
            let observations = observations.clone();
            move || {
                let file =
                    StudySettingsFile::new(&root_path, settings.clone(), crate::syscall::geteuid());
                match file {
                    Ok(file) => Box::new(
                        FileProfileHook::new(vec![file], spec, idle).with_mapping_loader(Box::new(
                            {
                                let observations = observations.clone();
                                move |files| {
                                    observations.lock().unwrap().push(files[0].read().unwrap());
                                    Ok(Default::default())
                                }
                            },
                        )),
                    ),
                    Err(_) => Box::new(
                        FileProfileHook::new(Vec::new(), spec, idle)
                            .with_unavailable(vec![settings.clone()]),
                    ),
                }
            }
        }));
        hook.recover().unwrap();
        let original = br#"{"version":1,"data":[{"group_name":"/graphics/raytracing","options":[{"name":"RayTracing","type":"bool","value":true},{"name":"RayTracedReflections","type":"bool","value":true},{"name":"RayTracedSunShadows","type":"bool","value":true},{"name":"RayTracedLocalShadows","type":"bool","value":true},{"name":"RayTracedPathTracing","type":"bool","value":false}]}]}"#;
        std::fs::write(&settings, original).unwrap();
        assert_eq!(hook.apply(Profile::Cpu), Ok(true));
        assert_ne!(std::fs::read(&settings).unwrap(), original);
        hook.restore().unwrap();
        assert_eq!(std::fs::read(&settings).unwrap(), original);
        // Simulate an interrupted earlier edit. Rediscovery must recover the durable snapshot
        // before asking the mapping loader to observe the file.
        let file = StudySettingsFile::new(&root_path, settings.clone(), crate::syscall::geteuid())
            .unwrap();
        let mut interrupted = FileProfileHook::new(vec![file], spec, idle);
        assert_eq!(interrupted.apply(Profile::Cpu), Ok(true));
        drop(interrupted);
        assert_eq!(hook.apply(Profile::Cpu), Ok(true));
        assert_eq!(
            observations.lock().unwrap().as_slice(),
            &[original.to_vec(), original.to_vec()]
        );
        hook.restore().unwrap();
        assert_eq!(std::fs::read(&settings).unwrap(), original);
    }

    #[test]
    fn mapping_discovery_blocks_unreadable_history_and_conflicting_arm_observations() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let uid = crate::syscall::geteuid();
        let mut files = Vec::new();
        for (name, index) in [("first", 4), ("second", 7)] {
            let dir = root_path.join(name);
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("UserSettings.json");
            let contents = format!(
                r#"{{"data":[{{"group_name":"/video/display","options":[{{"name":"Resolution","type":"string_list","value":"1920x1080","index":{index}}}]}}]}}"#
            );
            std::fs::write(&path, contents).unwrap();
            files.push(StudySettingsFile::new(&root_path, path, uid).unwrap());
        }
        assert_eq!(
            observed_mappings(&root_path, uid, &files),
            Err(ProfileFailure::Blocked("BlockedProfileMapping"))
        );
        let corrupt = root_path.join("profile-mappings-invalid.json");
        std::fs::write(&corrupt, b"{}").unwrap();
        std::fs::set_permissions(&corrupt, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            observed_mappings(&root_path, uid, &files[..1]),
            Err(ProfileFailure::Blocked("BlockedProfileMapping"))
        );
        std::fs::remove_file(&corrupt).unwrap();
        assert!(observed_mappings(&root_path, uid, &files[..1]).is_ok());
    }

    #[test]
    fn the_funnel_is_off_unless_both_switches_and_the_study_directory_are_set() {
        let mut config = config();
        assert!(select_funnel_builder(&config).is_none(), "off by default");
        config.research_study_dir = Some("/home/u/studies".into());
        config.research_child_funnel = true;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs live launch too"
        );
        config.research_child_funnel = false;
        config.research_live_launch = true;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs the funnel switch"
        );
        config.research_child_funnel = true;
        config.research_study_dir = None;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs the study directory"
        );
        config.research_study_dir = Some("/home/u/studies".into());
        assert!(select_funnel_builder(&config).is_some());
    }
}
