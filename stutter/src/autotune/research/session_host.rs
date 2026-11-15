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
    ResearchIntentV1,
    driver::AttemptHost,
    game_facts::ProcfsGameFacts,
    inspect_cyberpunk_installation,
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

/// The directories a study prefix places the game's native files in.
pub fn prefix_paths(study_dir: &Path, app_id: u32) -> (PathBuf, PathBuf, PathBuf) {
    let profile = study_dir
        .join("compatdata")
        .join(app_id.to_string())
        .join(PREFIX_USER);
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

/// The independent witnesses a restarted controller uses to retire an interrupted attempt. The
/// same real sources as the live host: the authenticated user manager (re-authenticated as
/// needed), procfs, and the worker's read-only token count.
pub fn build_recovery_witnesses(
    sources: &SessionHostSources,
    intent: &ResearchIntentV1,
) -> anyhow::Result<RecoveryWitnesses> {
    use super::{launch_txn::UserManagerPort, settlement::ProductionSettlementVerifier};
    let uid = crate::syscall::geteuid();
    let (_, _, settings) = prefix_paths(&sources.study_dir, intent.app_id);
    let installation = inspect_cyberpunk_installation(&sources.steamapps, uid)?;
    let executable = PathBuf::from(&installation.installation_directory).join(GAME_EXECUTABLE);
    let facts = game_facts_factory(uid, intent.app_id, intent.build_id, executable, settings);
    let mut tree_facts = facts();
    let mut observer = DbusUserManagerPort::new(ReauthenticatingBus::new(manager_authenticator(
        intent,
        &sources.steamapps,
        &sources.proton_manifest,
    )));
    let (token_socket, restored_socket, claim_socket) = (
        sources.worker_socket.clone(),
        sources.worker_socket.clone(),
        sources.worker_socket.clone(),
    );
    Ok(RecoveryWitnesses {
        unit_state: Box::new(move |unit| observer.unit_state(unit)),
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
    let (compat, results_dir, settings) = prefix_paths(&sources.study_dir, intent.app_id);
    verify_study_compatdata(&sources.steamapps, &sources.study_dir, &compat, uid, false)?;
    let (observed_steam_client, installed_tool_path) =
        inventory_lookup(&sources.steamapps, &sources.proton_manifest, uid)?;
    let installation = inspect_cyberpunk_installation(&sources.steamapps, uid)?;
    let tools = intent
        .tools
        .iter()
        .filter_map(|tool| installed_tool_path(tool).map(|path| (tool.source, path)))
        .collect();
    let executable = PathBuf::from(&installation.installation_directory).join(GAME_EXECUTABLE);
    let facts = game_facts_factory(uid, intent.app_id, intent.build_id, executable, settings);

    let send_bus = ReauthenticatingBus::new(manager_authenticator(
        intent,
        &sources.steamapps,
        &sources.proton_manifest,
    ));
    let observe_bus = ReauthenticatingBus::new(manager_authenticator(
        intent,
        &sources.steamapps,
        &sources.proton_manifest,
    ));
    let mut observer = DbusUserManagerPort::new(observe_bus);
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
            unit_state: Box::new(move |unit| {
                use super::launch_txn::UserManagerPort;
                observer.unit_state(unit)
            }),
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

    #[test]
    fn the_prefix_paths_are_derived_inside_the_study_directory() {
        let (compat, results, settings) = prefix_paths(Path::new("/study"), 1_091_500);
        assert_eq!(compat, Path::new("/study/compatdata"));
        assert!(results.starts_with("/study/compatdata/1091500"));
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
