//! Wall/monotonic clock discontinuity handling for the live autotune runtime.
//! Applies the existing `DaemonLifecyclePolicy`: a suspended interval makes the
//! pre-suspend measurement window incomparable and an active candidate is reverted.
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::AutotuneRuntime;
use crate::daemon::lifecycle::{
    DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs, DaemonLifecyclePolicy,
    DaemonLifecycleTransition, LifecycleClockSample, SuspendResumeDetector,
    evaluate_daemon_lifecycle_event,
};

/// Detector state plus the monotonic epoch its samples are measured from.
#[derive(Debug)]
pub(super) struct RuntimeLifecycle {
    policy: DaemonLifecyclePolicy,
    detector: SuspendResumeDetector,
    epoch: Instant,
    pub(super) last_transition: Option<DaemonLifecycleTransition>,
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        let policy = DaemonLifecyclePolicy::default();
        Self {
            detector: SuspendResumeDetector::with_policy(&policy),
            policy,
            epoch: Instant::now(),
            last_transition: None,
        }
    }
}

impl AutotuneRuntime {
    /// Samples the real clocks. `Instant` is CLOCK_MONOTONIC, which does not
    /// advance while suspended, whereas the wall clock does.
    pub(crate) fn poll_lifecycle_clock(&mut self) -> anyhow::Result<Option<DaemonLifecycleEvent>> {
        let realtime_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let monotonic_ms = self.lifecycle.epoch.elapsed().as_millis();
        self.observe_lifecycle_clock(LifecycleClockSample {
            realtime_ms,
            monotonic_ms,
        })
    }

    pub(crate) fn observe_lifecycle_clock(
        &mut self,
        sample: LifecycleClockSample,
    ) -> anyhow::Result<Option<DaemonLifecycleEvent>> {
        let Some(event) = self.lifecycle.detector.observe(sample) else {
            return Ok(None);
        };
        // A clock gap only reveals the resume. The suspend itself was missed, so apply
        // its policy (pause, flush, revert active candidate) before the resume policy.
        if matches!(event, DaemonLifecycleEvent::ResumeDetected { .. }) {
            self.handle_lifecycle_event(DaemonLifecycleEvent::SuspendDetected)?;
        }
        self.handle_lifecycle_event(event.clone())?;
        Ok(Some(event))
    }

    pub(crate) fn handle_lifecycle_event(
        &mut self,
        event: DaemonLifecycleEvent,
    ) -> anyhow::Result<()> {
        let has_active_experiment = self.live_experiments.has_active_experiment();
        let transition = evaluate_daemon_lifecycle_event(
            DaemonLifecycleInputs {
                has_active_experiment,
                active_experiment_has_rollback: has_active_experiment,
                event,
            },
            &self.lifecycle.policy,
        );
        let now = crate::audit::unix_nanos_now();
        log::warn!(
            "autotune_lifecycle_discontinuity event={} actions={:?}",
            transition.reason_code,
            transition.actions
        );
        // Always invalidate before any rollback decision, so nothing measured
        // before the discontinuity can be compared with later windows.
        if transition
            .actions
            .contains(&DaemonLifecycleAction::ClearMeasurementWindows)
        {
            self.controller.window.clear();
        }
        let rollback = transition
            .actions
            .contains(&DaemonLifecycleAction::RollbackActiveExperiment);
        let stabilization = transition.stabilization_ms;
        self.lifecycle.last_transition = Some(transition);
        if rollback {
            self.prepare_runtime_phase_for_evaluation()?;
            self.rollback_live_experiment(now, "system suspend/resume during active experiment")?;
        }
        if let Some(stabilization_ms) = stabilization {
            let until = now.saturating_add(u128::from(stabilization_ms) * 1_000_000);
            let state = &mut self.controller.state;
            state.cooldown_until_unix_nanos = Some(
                state
                    .cooldown_until_unix_nanos
                    .map_or(until, |existing| existing.max(until)),
            );
        }
        Ok(())
    }
}
