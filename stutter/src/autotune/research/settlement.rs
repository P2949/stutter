//! Production witnesses for claim settlement. Neither derives authority from the claim itself:
//! the process witness re-reads procfs for the recorded dispatchers and for any remaining game,
//! Wine, Proton or foreign Steam process; the worker witness asks an independent source how many
//! privileged transactions are still outstanding. A missing or failing source never settles.

use std::{fs::File, path::PathBuf};

use super::{
    dispatch_context::{DispatchContextV1, inspect_dispatcher_start},
    driver::{ClaimSettlementGuard, ClaimSettlementVerifier},
    launch_ticket::{ResearchLaunchTicketV1, SteamVerb},
    native_source::{GameFacts, Sighting},
    steam_port::host_has_game_or_foreign_steam,
};

/// Returns how many campaign-owned privileged transactions remain unsettled.
pub(crate) type OutstandingWitness = Box<dyn FnMut() -> anyhow::Result<usize> + Send>;

struct Settled;
impl ClaimSettlementGuard for Settled {}

pub(crate) struct ProductionSettlementVerifier {
    proc_root: PathBuf,
    outstanding: Option<OutstandingWitness>,
}

impl ProductionSettlementVerifier {
    pub(crate) fn new(proc_root: PathBuf, outstanding: Option<OutstandingWitness>) -> Self {
        Self {
            proc_root,
            outstanding,
        }
    }
}

impl ClaimSettlementVerifier for ProductionSettlementVerifier {
    fn verify_process(
        &mut self,
        dir: &File,
        context: &DispatchContextV1,
        ticket: &ResearchLaunchTicketV1,
    ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
        for verb in [SteamVerb::WaitForExitAndRun, SteamVerb::Run] {
            // A verb whose dispatcher never recorded a start has no process to account for;
            // an unreadable or inconsistent record is not a settlement.
            let recorded = super::report_store::read_private(
                dir,
                &format!("dispatcher-{}.json", verb.as_steam_argument()),
                context.intent.uid,
            );
            match recorded {
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error),
                Ok(_) => anyhow::ensure!(
                    inspect_dispatcher_start(dir, context, verb, &self.proc_root)?.is_none(),
                    "a recorded dispatcher is still alive"
                ),
            }
        }
        anyhow::ensure!(
            !host_has_game_or_foreign_steam(
                &self.proc_root,
                &ticket.steam_process,
                context.intent.uid
            ),
            "a game, Wine, Proton or foreign Steam process remains"
        );
        anyhow::ensure!(
            super::owned_domain::observe_domain(
                &self.proc_root,
                context.intent.uid,
                &context.spec.study_compat_data,
                std::process::id(),
                super::owned_domain::window_start_unix(dir)?,
            )?
            .is_clear(),
            "attempt-domain processes remain or cannot be attributed"
        );
        Ok(Box::new(Settled))
    }

    fn verify_worker(
        &mut self,
        _context: &DispatchContextV1,
    ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
        let witness = self
            .outstanding
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no independent worker-settlement witness"))?;
        anyhow::ensure!(
            witness()? == 0,
            "privileged transactions remain outstanding"
        );
        Ok(Box::new(Settled))
    }
}

/// The read-only owned-process-tree witness for completion. It reports settled only when no
/// recorded dispatcher is alive, the independent game observer sees no game, and the broad
/// Wine/Proton/Steam prefilter finds nothing beyond the frozen Steam client. Any read that
/// fails, or a missing client identity, is an error (unknown), never settled.
pub(crate) fn owned_process_tree_settled(
    dir: &File,
    context: &DispatchContextV1,
    proc_root: &std::path::Path,
    facts: &mut dyn GameFacts,
) -> anyhow::Result<bool> {
    for verb in [SteamVerb::WaitForExitAndRun, SteamVerb::Run] {
        let recorded = super::report_store::read_private(
            dir,
            &format!("dispatcher-{}.json", verb.as_steam_argument()),
            context.intent.uid,
        );
        match recorded {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
            Ok(_) => {
                if inspect_dispatcher_start(dir, context, verb, proc_root)?.is_some() {
                    return Ok(false);
                }
            }
        }
    }
    if facts.sight()? != Sighting::None {
        return Ok(false);
    }
    // The same attempt domain the cleanup uses: any attributed, unattributed or unknown member of
    // the study prefix keeps the tree unsettled, and an unobservable domain is an error.
    if !super::owned_domain::observe_domain(
        proc_root,
        context.intent.uid,
        &context.spec.study_compat_data,
        std::process::id(),
        super::owned_domain::window_start_unix(dir)?,
    )?
    .is_clear()
    {
        return Ok(false);
    }
    let client = context
        .intent
        .steam_client
        .process
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no frozen Steam client identity"))?;
    Ok(!host_has_game_or_foreign_steam(
        proc_root,
        client,
        context.intent.uid,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> DispatchContextV1 {
        use super::super::{AttemptSlot, dispatch_context::ArmSpec};
        let intent = super::super::tests::intent();
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        DispatchContextV1 {
            schema_version: 1,
            intent,
            slot,
            tickets: Default::default(),
            spec: ArmSpec {
                tool_dir: "/tools/GE".into(),
                study_compat_data: "/study/compatdata".into(),
                arm_env: Default::default(),
                steamapps: "/steam/steamapps".into(),
                proton_manifest: "/m.json".into(),
            },
        }
    }

    #[test]
    fn worker_witness_is_required_and_must_report_zero() {
        let ctx = context();
        let mut none = ProductionSettlementVerifier::new("/proc".into(), None);
        assert!(none.verify_worker(&ctx).is_err());
        let mut busy = ProductionSettlementVerifier::new("/proc".into(), Some(Box::new(|| Ok(1))));
        assert!(busy.verify_worker(&ctx).is_err());
        let mut failing = ProductionSettlementVerifier::new(
            "/proc".into(),
            Some(Box::new(|| Err(anyhow::anyhow!("worker unreachable")))),
        );
        assert!(failing.verify_worker(&ctx).is_err());
        let mut clear = ProductionSettlementVerifier::new("/proc".into(), Some(Box::new(|| Ok(0))));
        assert!(clear.verify_worker(&ctx).is_ok());
    }

    struct Facts(anyhow::Result<Sighting>);
    impl GameFacts for Facts {
        fn sight(&mut self) -> anyhow::Result<Sighting> {
            match &self.0 {
                Ok(sighting) => Ok(sighting.clone()),
                Err(_) => Err(anyhow::anyhow!("procfs unreadable")),
            }
        }
        fn settings_sha256(&mut self) -> anyhow::Result<String> {
            Ok("c".repeat(64))
        }
        fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
            Ok(1)
        }
    }

    fn add(proc: &std::path::Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
        let path = proc.join(pid.to_string());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            path.join("stat"),
            format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
        )
        .unwrap();
    }

    fn client_proc(ctx: &DispatchContextV1) -> tempfile::TempDir {
        let client = ctx.intent.steam_client.process.as_ref().unwrap();
        let proc = tempfile::tempdir().unwrap();
        add(
            proc.path(),
            client.steam_pid,
            1,
            client.steam_start_ticks,
            "steam",
        );
        add(
            proc.path(),
            client.webhelper_pid,
            client.steam_pid,
            client.webhelper_start_ticks,
            "steamwebhelper",
        );
        std::fs::write(proc.path().join("stat"), "btime 1000\n").unwrap();
        proc
    }

    fn attempt_dir() -> (tempfile::TempDir, File) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            tmp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(tmp.path().join("launch-requested"), b"x").unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        (tmp, dir)
    }

    #[test]
    fn the_tree_is_settled_only_with_no_game_no_busy_process_and_a_known_client() {
        let mut ctx = context();
        let uid = crate::syscall::geteuid();
        ctx.intent.uid = uid;
        ctx.intent.steam_client.process = Some(crate::autotune::research::SteamProcessIdentity {
            steam_pid: 100,
            steam_start_ticks: 11,
            webhelper_pid: 101,
            webhelper_start_ticks: 12,
            logged_in: true,
        });
        let proc = client_proc(&ctx);
        let (_tmp, dir) = attempt_dir();
        let settled = |facts: anyhow::Result<Sighting>, proc: &std::path::Path| {
            owned_process_tree_settled(&dir, &ctx, proc, &mut Facts(facts))
        };
        assert!(settled(Ok(Sighting::None), proc.path()).unwrap());
        // A game, an ambiguous sighting, or a busy Wine process is never settled.
        let identity = crate::autotune::research::cyberpunk::CyberpunkProcessIdentity {
            pid: stutter_core::ids::Pid::new(500),
            start_ticks: 9,
            uid,
            app_id: 1_091_500,
            build_id: 7,
            executable_sha256: "a".repeat(64),
        };
        assert!(!settled(Ok(Sighting::One(identity)), proc.path()).unwrap());
        assert!(!settled(Ok(Sighting::Multiple), proc.path()).unwrap());
        add(proc.path(), 300, 100, 20, "wineserver");
        assert!(!settled(Ok(Sighting::None), proc.path()).unwrap());
        // An unreadable observer, an unreadable /proc, or a missing client is unknown.
        assert!(settled(Err(anyhow::anyhow!("x")), proc.path()).is_err());
        assert!(
            settled(
                Ok(Sighting::None),
                std::path::Path::new("/definitely/not/proc")
            )
            .is_err()
        );
        ctx.intent.steam_client.process = None;
        assert!(
            owned_process_tree_settled(&dir, &ctx, proc.path(), &mut Facts(Ok(Sighting::None)))
                .is_err()
        );
    }
}
