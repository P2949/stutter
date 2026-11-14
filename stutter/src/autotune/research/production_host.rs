//! Assembles the production attempt host from the frozen intent and declared, independently
//! produced inputs. Construction qualifies first and fails closed: any identity that does not
//! match the frozen intent, a tool the intent names but no installation supplies, an unproven
//! entitlement, or a pending game update refuses to build a host. The built host still launches
//! only inside a qualified safe window and only through the authenticated transient launcher;
//! this constructor grants no launch authority and is not yet installed by the resident session.

use std::path::PathBuf;

use super::{
    ProtonSource, ResearchIntentV1, SteamUpdateStatus,
    driver::AttemptHost,
    launch_txn::{UnitState, UserManagerPort},
    native_probe::NativeResultProbe,
    native_source::{AttestedNativeSource, GameFacts},
    recovery_gather::CompletionWitness,
    settlement::owned_process_tree_settled,
    steam_host::SteamAttemptHost,
    steam_inventory::CyberpunkInstallation,
    steam_port::{PrepareRequest, ProductionSteamPort, TransientLauncher},
};

type FactsFactory = Box<dyn Fn() -> Box<dyn GameFacts + Send> + Send>;
type UnitObserver = Box<dyn FnMut(&str) -> anyhow::Result<UnitState> + Send>;
type TokenCount = Box<dyn FnMut() -> anyhow::Result<usize> + Send>;
type RestoredCheck = Box<dyn FnMut() -> anyhow::Result<bool> + Send>;

/// Everything the host needs that the intent alone cannot supply. Each is produced by an
/// independent source (Steam inventory, the user manager, procfs, the worker's read-only query).
pub struct ProductionHostInputs {
    pub study_compat_data: PathBuf,
    pub study_root: PathBuf,
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
    pub proc_root: PathBuf,
    pub tools: Vec<(ProtonSource, PathBuf)>,
    pub installation: CyberpunkInstallation,
    pub results_dir: PathBuf,
    /// Builds an independent game observer; called once for the native source and once for the
    /// process-tree witness so neither shares state with the other.
    pub facts: FactsFactory,
    pub manager: Box<dyn UserManagerPort + Send>,
    pub prepare: PrepareRequest,
    pub unit_state: UnitObserver,
    pub outstanding_worker_tokens: TokenCount,
    pub transactions_restored: RestoredCheck,
}

fn qualify(intent: &ResearchIntentV1, inputs: &ProductionHostInputs) -> anyhow::Result<()> {
    intent.validate()?;
    let installation = &inputs.installation;
    anyhow::ensure!(
        intent.uid == crate::syscall::geteuid()
            && installation.app_id == intent.app_id
            && installation.build_id == intent.build_id
            && installation.game_executable_sha256.as_deref()
                == Some(intent.game_executable_sha256.as_str()),
        "BlockedHostIdentity: the installed game differs from the frozen intent"
    );
    anyhow::ensure!(
        installation.entitlement_verified
            && installation.manifest_update_status == SteamUpdateStatus::NoPendingUpdateIndicated,
        "BlockedHostIdentity: entitlement or update state is not frozen"
    );
    for tool in &intent.tools {
        anyhow::ensure!(
            inputs
                .tools
                .iter()
                .any(|(source, _)| *source == tool.source),
            "BlockedHostIdentity: a frozen tool has no verified installation"
        );
    }
    anyhow::ensure!(
        inputs
            .tools
            .iter()
            .all(|(source, _)| intent.tools.iter().any(|tool| tool.source == *source)),
        "BlockedHostIdentity: an installed tool is outside the frozen set"
    );
    Ok(())
}

pub fn build_production_host(
    intent: &ResearchIntentV1,
    inputs: ProductionHostInputs,
) -> anyhow::Result<Box<dyn AttemptHost + Send>> {
    qualify(intent, &inputs)?;
    let ProductionHostInputs {
        study_compat_data,
        study_root,
        steamapps,
        proton_manifest,
        proc_root,
        tools,
        installation,
        results_dir,
        facts,
        manager,
        prepare,
        unit_state,
        outstanding_worker_tokens,
        transactions_restored,
    } = inputs;
    let source = AttestedNativeSource::new(facts(), installation, intent.tools.clone(), intent.uid);
    let probe = NativeResultProbe::new(results_dir, intent.uid, source);
    let mut tree_facts = facts();
    let witness = CompletionWitness {
        unit_state,
        process_tree_settled: Box::new(move |dir, context| {
            owned_process_tree_settled(dir, context, &proc_root, &mut *tree_facts)
        }),
        outstanding_worker_tokens,
        transactions_restored,
    };
    let port = ProductionSteamPort::new(
        study_compat_data,
        study_root,
        steamapps,
        proton_manifest,
        intent.steam_client.clone(),
        tools,
        probe,
    )
    .with_transient_launcher(TransientLauncher { manager, prepare })
    .with_settlement_witness(Box::new(witness));
    Ok(Box::new(SteamAttemptHost::new(port)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::research::{
        launch_txn::SendError, native_source::Sighting, user_launcher::UserLaunchRequest,
    };

    struct Facts;
    impl GameFacts for Facts {
        fn sight(&mut self) -> anyhow::Result<Sighting> {
            Ok(Sighting::None)
        }
        fn settings_sha256(&mut self) -> anyhow::Result<String> {
            Ok("c".repeat(64))
        }
        fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
            Ok(1)
        }
    }

    struct Manager;
    impl UserManagerPort for Manager {
        fn send_start(&mut self, _: &UserLaunchRequest) -> Result<(), SendError> {
            panic!("construction must never send a launch request")
        }
        fn unit_state(&mut self, _: &str) -> anyhow::Result<UnitState> {
            Ok(UnitState::Absent)
        }
        fn proves_never_executable(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    fn intent() -> ResearchIntentV1 {
        let mut intent = crate::autotune::research::tests::intent();
        intent.uid = crate::syscall::geteuid();
        intent.game_executable_sha256 = "a".repeat(64);
        intent
    }

    fn inputs(intent: &ResearchIntentV1) -> ProductionHostInputs {
        ProductionHostInputs {
            study_compat_data: "/s".into(),
            study_root: "/study".into(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
            proc_root: "/proc".into(),
            tools: intent
                .tools
                .iter()
                .map(|tool| (tool.source, PathBuf::from("/t")))
                .collect(),
            installation: CyberpunkInstallation {
                app_id: intent.app_id,
                build_id: intent.build_id,
                installation_directory: "x".into(),
                manifest_update_status: SteamUpdateStatus::NoPendingUpdateIndicated,
                manifest_sha256: "d".repeat(64),
                game_executable_sha256: Some(intent.game_executable_sha256.clone()),
                launcher_executable_sha256: None,
                entitlement_verified: true,
                native_benchmark_verified: false,
            },
            results_dir: "/results".into(),
            facts: Box::new(|| Box::new(Facts)),
            manager: Box::new(Manager),
            prepare: Box::new(|_, _| anyhow::bail!("preparation must not run at construction")),
            unit_state: Box::new(|_| Ok(UnitState::Absent)),
            outstanding_worker_tokens: Box::new(|| Ok(0)),
            transactions_restored: Box::new(|| Ok(true)),
        }
    }

    #[test]
    fn a_matching_intent_builds_a_host_that_is_not_yet_in_a_safe_window() {
        let intent = intent();
        let mut host = build_production_host(&intent, inputs(&intent)).unwrap();
        // Construction sent nothing, prepared nothing, and no idle window has been observed.
        assert!(!host.safe_window());
    }

    #[test]
    fn any_identity_or_tool_mismatch_refuses_to_build() {
        let base = intent();
        let cases: [fn(&mut ProductionHostInputs); 7] = [
            |i| i.installation.build_id += 1,
            |i| i.installation.game_executable_sha256 = Some("b".repeat(64)),
            |i| i.installation.entitlement_verified = false,
            |i| i.installation.manifest_update_status = SteamUpdateStatus::Updating,
            |i| i.installation.manifest_update_status = SteamUpdateStatus::Unknown,
            |i| {
                i.tools.pop();
            },
            |i| i.tools.push((ProtonSource::Em, "/t".into())),
        ];
        for mutate in cases {
            let mut broken = inputs(&base);
            mutate(&mut broken);
            let error = build_production_host(&base, broken)
                .err()
                .expect("must refuse");
            assert!(
                error.to_string().contains("BlockedHostIdentity"),
                "{error:#}"
            );
        }
        let mut other_owner = base.clone();
        other_owner.uid = crate::syscall::geteuid() + 1;
        assert!(build_production_host(&other_owner, inputs(&other_owner)).is_err());
    }
}
