//! Rollback handler for `PmQosRelease` tokens.
//!
//! A PM QoS lease ends when its holder's descriptor closes, so recovery never writes. It reads
//! the aggregate target and counts a lease as released only when the target no longer equals the
//! request. A target still equal to it may be an identical request from another holder, so it is
//! reported as not proven and never as restored.
use super::{
    ActionBoundaryError, RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
    RollbackToken, pm_qos_transaction::PmQosReleaseRecord,
};

#[derive(Default)]
pub(crate) struct PmQosRollbackHandler;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReleaseCheck {
    pub released: usize,
    pub unproven: usize,
    pub unreadable: usize,
    pub messages: Vec<String>,
}

pub(crate) fn check_release(records: &[PmQosReleaseRecord]) -> ReleaseCheck {
    let mut check = ReleaseCheck::default();
    for record in records {
        match super::pm_qos::read_effective(&record.device) {
            Ok(effective) if effective != record.requested_us => check.released += 1,
            Ok(effective) => {
                check.unproven += 1;
                check.messages.push(format!(
                    "aggregate target {effective} us still equals the released request; holder not proven gone"
                ));
            }
            Err(error) => {
                check.unreadable += 1;
                check
                    .messages
                    .push(format!("cannot read {}: {error}", record.device.display()));
            }
        }
    }
    check
}

impl RollbackHandler for PmQosRollbackHandler {
    fn id(&self) -> &'static str {
        "pm-qos-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "pm-qos-release")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "pm-qos-release")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::PmQosRelease { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        anyhow::ensure!(
            self.supports_token(token),
            "unsupported token {}",
            token.kind()
        );
        Ok(RollbackPreview {
            handler_id: self.id(),
            restore_path: Default::default(),
            affected_tasks: token.affected_tasks(),
            message: "would read the PM QoS aggregate target to prove the lease is released".into(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::PmQosRelease { records } = token else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "pm-qos-release",
                token.kind(),
            )
            .into());
        };
        let check = check_release(records);
        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: Default::default(),
            restored: check.released,
            skipped_dead: check.unproven,
            skipped_identity_mismatch: 0,
            skipped_externally_changed: 0,
            legacy_unverified: 0,
            errors: check.unreadable,
            messages: check.messages,
        })
    }
}
