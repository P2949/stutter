use super::*;
use crate::daemon::state::DaemonState;

impl AutotuneRuntime {
    pub fn rollback_on_stop(&mut self, reason: &str) -> anyhow::Result<Option<DaemonState>> {
        if !self.has_active_experiment() {
            return Ok(None);
        }

        self.rollback_live_experiment(crate::audit::unix_nanos_now(), reason)?;

        Ok(Some(self.daemon_state_snapshot()))
    }
}
