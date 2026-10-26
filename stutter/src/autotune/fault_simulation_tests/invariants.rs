//! The invariants every simulated event must leave intact (PLAN M9 section 12.4).
use std::path::PathBuf;

use super::Sim;
use crate::autotune::{
    autonomous_support::AUTONOMOUS_CONTRACTS,
    controller_journal::read_controller_journal,
    kept_ledger::{kept_ledger_path, read_kept_ledger},
};

impl Sim {
    /// Tokens the controller can still find after a crash: only what is on disk counts.
    pub(super) fn durable_tokens(&self) -> Vec<PathBuf> {
        let journal = read_controller_journal(&self.journal_path())
            .ok()
            .and_then(|record| {
                record
                    .rollback_token()
                    .and_then(|token| token.restore_path().cloned())
            });
        let ledger = read_kept_ledger(&kept_ledger_path(&self.journal_path()))
            .map(|ledger| ledger.entries)
            .unwrap_or_default();
        journal
            .into_iter()
            .chain(
                ledger
                    .iter()
                    .filter_map(|entry| entry.rollback_token.restore_path().cloned()),
            )
            .collect()
    }

    pub(super) fn check(&self) -> Result<(), String> {
        let host = self.host.borrow();
        let durable = self.durable_tokens();

        // 0 unrecovered rollback tokens: every effect still live on the host has a durable token.
        for (path, effect) in host.live_effects() {
            if !durable.contains(path) {
                return Err(format!(
                    "unrecovered token: {:?} effect {} is live on the host but not durable (journal holds {durable:?})",
                    effect.group,
                    path.display()
                ));
            }
        }

        // 0 simultaneous conflicting action groups.
        let live: Vec<_> = host
            .live_effects()
            .map(|(_, effect)| effect.group)
            .collect();
        for (index, group) in live.iter().enumerate() {
            if live[index + 1..]
                .iter()
                .any(|other| group.conflicts_with(*other))
            {
                return Err(format!("conflicting groups live together: {live:?}"));
            }
        }
        let kept: Vec<_> = self
            .profile
            .kept_actions()
            .map(|(group, _)| *group)
            .collect();
        for (index, group) in kept.iter().enumerate() {
            if kept[index + 1..]
                .iter()
                .any(|other| group.conflicts_with(*other))
            {
                return Err(format!("conflicting kept groups: {kept:?}"));
            }
        }

        // 0 apply outside the Supported contract matrix.
        for tuple in host
            .applied_tuples
            .iter()
            .filter(|tuple| *tuple != "test-only-fake")
        {
            let supported = AUTONOMOUS_CONTRACTS.iter().any(|contract| {
                tuple.starts_with(&format!(
                    "{:?}/{:?}/",
                    contract.operation, contract.objective
                )) && tuple.ends_with("GameCpuSchedulerPressure")
            });
            if !supported {
                return Err(format!("applied outside the support matrix: {tuple}"));
            }
        }

        // Every mutation is accounted for by an audited transition.
        if host.mutations > self.audited_mutations {
            return Err(format!(
                "{} host mutation(s) but only {} audited",
                host.mutations, self.audited_mutations
            ));
        }

        if let Some(violation) = self.violations.first() {
            return Err(violation.clone());
        }
        Ok(())
    }
}
