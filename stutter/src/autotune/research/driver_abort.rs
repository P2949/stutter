//! Stopping an attempt without blocking the controller. A failure, timeout, budget stop or cancellation used to call
//! the host's blocking `abort` (for Steam, up to the whole settlement window) from inside the tick, stalling the
//! resident event loop. The host is now asked one bounded step per tick: while its settlement window is open the tick
//! reports the attempt still running and the attempt stays active (its time still accrues); the outcome is sealed only
//! when the independent witness proves the launched tree gone, exactly as before. An unproven settlement is still an
//! error and never a seal. A host with nothing to wait for keeps the blocking behaviour through the default step.

use super::{
    super::{
        journal::{AttemptOutcome, AttemptSlot},
        steam_host::StopStep,
    },
    AttemptHost, Driver, FailureClass, Step,
};

/// An attempt whose stop is in progress, with the outcome it will be sealed with once it is proven settled.
#[derive(Debug)]
pub(super) struct PendingAbort {
    slot: AttemptSlot,
    outcome: AttemptOutcome,
    terminal: Option<FailureClass>,
}

impl Driver {
    /// Cancel-time ticks to proven settlement: an explicit shutdown or cancellation may wait for the settlement window
    /// (bounded), unlike an ordinary tick, so the settlement is proven or reported unproven before it returns.
    pub fn tick_cancelled_to_settlement(
        &mut self,
        host: &mut dyn AttemptHost,
        now: u64,
    ) -> anyhow::Result<Step> {
        const LONGEST: std::time::Duration = std::time::Duration::from_secs(150);
        let began = std::time::Instant::now();
        let mut result = self.tick_with_launch_fence(host, now, || {
            anyhow::bail!("cancellation must not launch an attempt")
        });
        while matches!(result, Ok(Step::Running(_)))
            && self.abort_pending()
            && began.elapsed() < LONGEST
        {
            std::thread::sleep(std::time::Duration::from_secs(1));
            result = self.tick_with_launch_fence(host, now, || {
                anyhow::bail!("cancellation must not launch an attempt")
            });
        }
        result
    }

    /// Whether an attempt is being stopped (its settlement is not yet proven).
    pub fn abort_pending(&self) -> bool {
        self.pending_abort.is_some()
    }

    /// Stop the attempt and seal `outcome` once it is proven settled; while the settlement window is open the tick
    /// reports the attempt running and a later tick continues.
    pub(super) fn abort_then_seal(
        &mut self,
        host: &mut dyn AttemptHost,
        slot: AttemptSlot,
        outcome: AttemptOutcome,
        terminal: Option<FailureClass>,
        now: u64,
    ) -> anyhow::Result<Step> {
        match host.abort_step() {
            StopStep::Settled => self.seal_settled(slot, outcome, terminal, now),
            StopStep::Pending => {
                let index = slot.index;
                self.pending_abort = Some(PendingAbort {
                    slot,
                    outcome,
                    terminal,
                });
                Ok(Step::Running(index))
            }
            StopStep::Unsettled => anyhow::bail!("settlement unproven"),
        }
    }

    /// Continue a stop begun on an earlier tick, if one is in progress.
    pub(super) fn continue_abort(
        &mut self,
        host: &mut dyn AttemptHost,
        now: u64,
    ) -> Option<anyhow::Result<Step>> {
        let pending = self.pending_abort.take()?;
        Some(match host.abort_step() {
            StopStep::Settled => {
                self.seal_settled(pending.slot, pending.outcome, pending.terminal, now)
            }
            StopStep::Pending => {
                let index = pending.slot.index;
                self.pending_abort = Some(pending);
                Ok(Step::Running(index))
            }
            StopStep::Unsettled => {
                // Still owned: it stays pending so nothing seals over a possibly running game.
                self.pending_abort = Some(pending);
                Err(anyhow::anyhow!("settlement unproven"))
            }
        })
    }

    fn seal_settled(
        &mut self,
        slot: AttemptSlot,
        outcome: AttemptOutcome,
        terminal: Option<FailureClass>,
        now: u64,
    ) -> anyhow::Result<Step> {
        let sealed = self.seal(slot, outcome, now)?;
        if let Some(class) = terminal {
            self.terminal_failure = Some(class);
        }
        Ok(sealed)
    }
}
