//! Production witnesses for claim settlement. Neither derives authority from the claim itself:
//! the process witness re-reads procfs for the recorded dispatchers and for any remaining game,
//! Wine, Proton or foreign Steam process; the worker witness asks an independent source how many
//! privileged transactions are still outstanding. A missing or failing source never settles.

use std::{fs::File, path::PathBuf};

use super::{
    dispatch_context::{DispatchContextV1, inspect_dispatcher_start},
    driver::{ClaimSettlementGuard, ClaimSettlementVerifier},
    launch_ticket::{ResearchLaunchTicketV1, SteamVerb},
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
}
