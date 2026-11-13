//! Passive campaign session identity and observation; this grants no execution authority.
use super::ResearchCoordinator;
use crate::autotune::research::desktop_idle::DesktopIdleObservation;

impl ResearchCoordinator {
    pub(in crate::autotune::runtime) fn desktop_idle_target(
        &self,
    ) -> Option<(u32, String, String)> {
        let active = self.active.as_ref()?;
        let facts = self.facts.as_ref()?;
        Some((
            facts.uid,
            facts.session_id.clone(),
            active.intent_sha256.clone(),
        ))
    }

    pub(in crate::autotune::runtime) fn set_desktop_idle(
        &mut self,
        observation: (std::time::Instant, DesktopIdleObservation),
    ) {
        self.desktop_idle = observation;
    }
}
