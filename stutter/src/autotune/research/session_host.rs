//! Builds the resident session's production attempt host from real, independently inspected
//! sources: the study prefix, Steam's inventory and installation manifest, the authenticated user
//! manager, the privileged worker's read-only token count, and procfs. Every input that cannot be
//! produced is an error, so an incomplete host is never built.
//!
//! The paths derived inside the Wine prefix and the game's executable location below are
//! assumptions that have not been checked against a real Cyberpunk-under-Proton launch. They are
//! named, documented and fail closed: a missing or different file refuses to build, or makes the
//! observer report a mismatch, and never lets an attempt proceed.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    CyberpunkInstallation, ResearchIntentV1, discover_cyberpunk_installation,
    discover_steam_libraries,
    driver::AttemptHost,
    game_facts::ProcfsGameFacts,
    launch_txn::UnitState,
    native_source::GameFacts,
    prefix::verify_study_compatdata,
    production_host::{ProductionHostInputs, build_production_host, inventory_lookup},
    recovery_gather::RecoveryWitnesses,
    settlement::owned_process_tree_settled,
    user_launcher::{VerifiedSessionEnvironment, prepare_from_host},
    user_manager::VerifiedUserManager,
    user_manager_port::{DbusUserManagerPort, ReauthenticatingBus},
};
use crate::daemon::privilege::UnixSocketPrivilegedActionService;

/// Where the game's executable sits under the Steam installation directory (unverified).
const GAME_EXECUTABLE: &str = "bin/x64/Cyberpunk2077.exe";
/// The Wine user profile inside the study prefix (unverified for Proton prefixes).
const PREFIX_USER: &str = "pfx/drive_c/users/steamuser";
/// The native benchmark results directory under the Wine profile (unverified).
const BENCHMARK_RESULTS: &str = "Documents/CD Projekt Red/Cyberpunk 2077/benchmarkResults";
/// The game's user settings file under the Wine profile (unverified).
const USER_SETTINGS: &str = "AppData/Local/CD Projekt Red/Cyberpunk 2077/UserSettings.json";

/// The configured sources the session already has.
#[derive(Clone)]
pub struct SessionHostSources {
    pub study_dir: PathBuf,
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
    pub worker_socket: Option<PathBuf>,
}

/// The directories a study prefix places the game's native files in. Steam's data path is the study
/// `compatdata` directory itself (the dispatcher substitutes it), and Proton creates its prefix
/// directly below it, so the profile is `compatdata/pfx/…` with no application-id directory.
pub fn prefix_paths(study_dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let profile = study_dir.join("compatdata").join(PREFIX_USER);
    (
        study_dir.join("compatdata"),
        profile.join(BENCHMARK_RESULTS),
        profile.join(USER_SETTINGS),
    )
}

fn manager_authenticator(
    intent: &ResearchIntentV1,
    steamapps: &Path,
    manifest: &Path,
) -> impl FnMut() -> anyhow::Result<VerifiedUserManager> + use<> {
    let (intent, steamapps, manifest) = (intent.clone(), steamapps.to_owned(), manifest.to_owned());
    move || {
        let session = VerifiedSessionEnvironment::inspect(&intent, &steamapps, &manifest)?;
        VerifiedUserManager::inspect(&session, &intent)
    }
}

type UnitStateFn = Box<dyn FnMut(&str) -> anyhow::Result<UnitState> + Send>;
type UnitInvocationFn = Box<dyn FnMut(&str) -> anyhow::Result<Option<String>> + Send>;

/// The unit-state and invocation-id observations, both through one re-authenticating manager
/// connection so they always describe the same unit.
fn unit_witnesses(
    intent: &ResearchIntentV1,
    steamapps: &Path,
    manifest: &Path,
) -> (UnitStateFn, UnitInvocationFn) {
    use std::sync::{Arc, Mutex};

    use super::launch_txn::UserManagerPort;
    let port = Arc::new(Mutex::new(DbusUserManagerPort::new(
        ReauthenticatingBus::new(manager_authenticator(intent, steamapps, manifest)),
    )));
    let state_port = Arc::clone(&port);
    (
        Box::new(move |unit| {
            state_port
                .lock()
                .map_err(|_| anyhow::anyhow!("user manager observer is poisoned"))?
                .unit_state(unit)
        }),
        Box::new(move |unit| {
            port.lock()
                .map_err(|_| anyhow::anyhow!("user manager observer is poisoned"))?
                .unit_invocation(unit)
        }),
    )
}

fn worker_tokens(socket: &Option<PathBuf>) -> anyhow::Result<usize> {
    let socket = socket
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("BlockedHostIdentity: no privileged worker socket"))?;
    UnixSocketPrivilegedActionService::new(socket.clone()).outstanding_tokens()
}

fn game_facts_factory(
    uid: u32,
    app_id: u32,
    build_id: u64,
    executable: PathBuf,
    settings: PathBuf,
) -> impl Fn() -> Box<dyn GameFacts + Send> + Send + 'static {
    move || {
        let settings = settings.clone();
        Box::new(ProcfsGameFacts::new(
            PathBuf::from("/proc"),
            uid,
            app_id,
            build_id,
            executable.clone(),
            Box::new(move || {
                let bytes = std::fs::read(&settings)?;
                anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "settings file is too large");
                Ok(stutter_core::hash::sha256_hex(&bytes))
            }),
            Box::new(|| Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())),
        ))
    }
}

/// The installed game and its absolute executable, taken from the registered library that actually
/// holds the manifest. Steam records `installdir` as a bare directory name below that library's
/// `steamapps/common`, so the executable is never derived from the primary library or the bare name.
fn verified_game(
    primary_steamapps: &Path,
    uid: u32,
) -> anyhow::Result<(CyberpunkInstallation, PathBuf)> {
    let libraries = discover_steam_libraries(primary_steamapps, uid)?;
    let (library, installation) = discover_cyberpunk_installation(&libraries, uid)?;
    let executable = library
        .join("common")
        .join(&installation.installation_directory)
        .join(GAME_EXECUTABLE);
    anyhow::ensure!(
        executable.is_absolute(),
        "the game executable is not an absolute path"
    );
    Ok((installation, executable))
}

/// The independent witnesses a restarted controller uses to retire an interrupted attempt. The
/// same real sources as the live host: the authenticated user manager (re-authenticated as
/// needed), procfs, and the worker's read-only token count.
pub fn build_recovery_witnesses(
    sources: &SessionHostSources,
    intent: &ResearchIntentV1,
) -> anyhow::Result<RecoveryWitnesses> {
    use super::settlement::ProductionSettlementVerifier;
    let uid = crate::syscall::geteuid();
    let (_, _, settings) = prefix_paths(&sources.study_dir);
    let (_, executable) = verified_game(&sources.steamapps, uid)?;
    let facts = game_facts_factory(uid, intent.app_id, intent.build_id, executable, settings);
    let mut tree_facts = facts();
    let (unit_state, unit_invocation) =
        unit_witnesses(intent, &sources.steamapps, &sources.proton_manifest);
    let (token_socket, restored_socket, claim_socket) = (
        sources.worker_socket.clone(),
        sources.worker_socket.clone(),
        sources.worker_socket.clone(),
    );
    Ok(RecoveryWitnesses {
        unit_state,
        unit_invocation,
        process_tree_settled: Box::new(move |dir, context| {
            owned_process_tree_settled(dir, context, Path::new("/proc"), &mut *tree_facts)
        }),
        outstanding_worker_tokens: Box::new(move || worker_tokens(&token_socket)),
        transactions_restored: Box::new(move || Ok(worker_tokens(&restored_socket)? == 0)),
        claim_verifier: Box::new(ProductionSettlementVerifier::new(
            PathBuf::from("/proc"),
            Some(Box::new(move || worker_tokens(&claim_socket))),
        )),
    })
}

/// Assemble the host for one frozen intent. The study prefix, Steam inventory and installation
/// are checked first; any missing source is an error and nothing is constructed.
pub fn build_session_host(
    sources: &SessionHostSources,
    intent: &ResearchIntentV1,
) -> anyhow::Result<Box<dyn AttemptHost + Send>> {
    let uid = crate::syscall::geteuid();
    let (compat, results_dir, settings) = prefix_paths(&sources.study_dir);
    verify_study_compatdata(&sources.steamapps, &sources.study_dir, &compat, uid, false)?;
    let (observed_steam_client, installed_tool_path) =
        inventory_lookup(&sources.steamapps, &sources.proton_manifest, uid)?;
    let (mut installation, executable) = verified_game(&sources.steamapps, uid)?;
    // The inventory cannot prove entitlement; the signed-in account's own launch record is the
    // preflight evidence, and anything unreadable leaves it false so no host is built.
    installation.entitlement_verified = match (
        sources.steamapps.parent(),
        observed_steam_client
            .as_ref()
            .and_then(|client| client.process.as_ref()),
    ) {
        (Some(steam_root), Some(process)) => super::entitlement::authenticated_account_has_played(
            Path::new("/proc"),
            steam_root,
            process,
            intent.app_id,
        ),
        _ => false,
    };
    let tools = intent
        .tools
        .iter()
        .filter_map(|tool| installed_tool_path(tool).map(|path| (tool.source, path)))
        .collect();
    let facts = game_facts_factory(uid, intent.app_id, intent.build_id, executable, settings);

    let send_bus = ReauthenticatingBus::new(manager_authenticator(
        intent,
        &sources.steamapps,
        &sources.proton_manifest,
    ));
    let (unit_state, unit_invocation) =
        unit_witnesses(intent, &sources.steamapps, &sources.proton_manifest);
    let (steamapps, manifest) = (sources.steamapps.clone(), sources.proton_manifest.clone());
    let (token_socket, restored_socket) =
        (sources.worker_socket.clone(), sources.worker_socket.clone());
    build_production_host(
        intent,
        ProductionHostInputs {
            study_compat_data: compat,
            study_root: sources.study_dir.clone(),
            steamapps: sources.steamapps.clone(),
            proton_manifest: sources.proton_manifest.clone(),
            proc_root: PathBuf::from("/proc"),
            tools,
            installation,
            observed_steam_client,
            installed_tool_path,
            results_dir,
            facts: Box::new(facts),
            manager: Box::new(DbusUserManagerPort::new(send_bus)),
            prepare: Box::new(move |slot, context| {
                prepare_from_host(&context.intent, slot, &steamapps, &manifest)
            }),
            unit_state,
            unit_invocation,
            outstanding_worker_tokens: Box::new(move || worker_tokens(&token_socket)),
            // Restoration is the worker reporting no issued-and-unrestored transaction.
            transactions_restored: Box::new(move || Ok(worker_tokens(&restored_socket)? == 0)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(root: &Path) -> SessionHostSources {
        SessionHostSources {
            study_dir: root.join("study"),
            steamapps: root.join("steamapps"),
            proton_manifest: root.join("manifest.json"),
            worker_socket: None,
        }
    }

    /// A Steam layout with the game in a secondary library, as on a real multi-drive install.
    fn secondary_library_layout(root: &Path) -> PathBuf {
        let primary = root.join("primary");
        let secondary = root.join("secondary");
        std::fs::create_dir_all(primary.join("steamapps")).unwrap();
        let bin = secondary.join("steamapps/common/Cyberpunk 2077/bin/x64");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("Cyberpunk2077.exe"), b"MZfixture").unwrap();
        std::fs::write(
            secondary.join("steamapps/appmanifest_1091500.acf"),
            "\"AppState\" { \"appid\" \"1091500\" \"buildid\" \"42\" \"installdir\" \"Cyberpunk 2077\" \"StateFlags\" \"4\" \"TargetBuildID\" \"0\" \"BytesToDownload\" \"2\" \"BytesDownloaded\" \"2\" \"BytesToStage\" \"3\" \"BytesStaged\" \"3\" \"InstalledDepots\" { \"1\" { \"appid\" \"other\" } } }",
        )
        .unwrap();
        std::fs::write(
            primary.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
                primary.display(),
                secondary.display()
            ),
        )
        .unwrap();
        primary.join("steamapps")
    }

    #[test]
    fn the_executable_comes_from_the_library_that_holds_the_game_and_is_absolute() {
        let root = tempfile::tempdir().unwrap();
        let steamapps = secondary_library_layout(root.path());
        let uid = crate::syscall::geteuid();
        let (installation, executable) = verified_game(&steamapps, uid).unwrap();
        assert_eq!(installation.installation_directory, "Cyberpunk 2077");
        assert_eq!(
            executable,
            root.path()
                .join("secondary/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe")
        );
        // The real consumer accepts it: with no game running the observer reports absence rather
        // than failing on the path, which is what the former relative path caused.
        let facts = game_facts_factory(uid, 1_091_500, 42, executable, root.path().join("s.json"));
        assert!(facts().sight().is_ok());
        let relative = game_facts_factory(
            uid,
            1_091_500,
            42,
            PathBuf::from("Cyberpunk 2077").join(GAME_EXECUTABLE),
            root.path().join("s.json"),
        );
        assert!(relative().sight().is_err());
    }
    #[test]
    fn the_prefix_paths_are_derived_inside_the_study_directory() {
        let (compat, results, settings) = prefix_paths(Path::new("/study"));
        assert_eq!(compat, Path::new("/study/compatdata"));
        assert!(results.starts_with("/study/compatdata/pfx/drive_c/users/steamuser"));
        assert!(results.ends_with("benchmarkResults"));
        assert!(settings.ends_with("UserSettings.json"));
    }

    #[test]
    fn missing_sources_refuse_to_build_and_touch_nothing() {
        let root = tempfile::tempdir().unwrap();
        let intent = crate::autotune::research::tests::intent();
        let error = build_session_host(&sources(root.path()), &intent)
            .err()
            .expect("a host must not be built without its sources");
        assert!(!error.to_string().is_empty());
        // Nothing was created under the missing study directory.
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_worker_socket_is_an_error_never_zero_tokens() {
        assert!(worker_tokens(&None).is_err());
    }
}
