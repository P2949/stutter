use super::*;
use crate::daemon::state::DaemonState;

impl AutotuneRuntime {
    /// A stop leaves nothing applied: the active experiment reverts and every kept action rolls
    /// back. A kept action that cannot be restored stays in the durable ledger for startup
    /// recovery, and the stop reports it as a failure.
    pub fn rollback_on_stop(&mut self, reason: &str) -> anyhow::Result<Option<DaemonState>> {
        if !self.has_active_experiment() && !self.has_kept_actions() {
            return Ok(None);
        }

        let experiment_result = if self.has_active_experiment() {
            self.rollback_live_experiment(crate::audit::unix_nanos_now(), reason)
        } else {
            Ok(())
        };
        let failed = self.release_kept_actions(reason)?;
        experiment_result?;
        anyhow::ensure!(
            failed == 0,
            "{failed} kept action(s) could not be restored on stop; the kept-action ledger retains \
             their rollback tokens for startup recovery (or `stutter daemon emergency-restore`)"
        );

        Ok(Some(self.daemon_state_snapshot()))
    }
}
