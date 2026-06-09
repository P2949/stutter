use std::fmt;

use super::AutotuneRuntime;
use crate::autotune::state::ControllerPhase;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum AutotuneRuntimePhase {
    #[default]
    Idle,
    ObservingBaseline,
    Planning,
    DryRun,
    ApplyingCandidate,
    MeasuringCandidate,
    KeepingCandidate,
    RevertingCandidate,
    Faulted,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AutotuneRuntimePhaseMachine {
    phase: AutotuneRuntimePhase,
    fault_reason: Option<String>,
}

impl AutotuneRuntimePhaseMachine {
    pub(crate) fn phase(&self) -> AutotuneRuntimePhase {
        self.phase
    }

    pub(crate) fn transition(
        &mut self,
        next: AutotuneRuntimePhase,
        reason: impl Into<String>,
    ) -> Result<(), AutotuneRuntimeTransitionError> {
        let reason = reason.into();
        if self.phase.allows_transition_to(next) {
            self.phase = next;
            if next != AutotuneRuntimePhase::Faulted {
                self.fault_reason = None;
            }
            return Ok(());
        }

        Err(AutotuneRuntimeTransitionError {
            from: self.phase,
            to: next,
            reason,
        })
    }

    pub(crate) fn force_fault(&mut self, reason: impl Into<String>) {
        self.phase = AutotuneRuntimePhase::Faulted;
        self.fault_reason = Some(reason.into());
    }
}

impl AutotuneRuntimePhase {
    pub(crate) fn allows_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        if next == Self::Faulted {
            return true;
        }

        match (self, next) {
            (Self::Idle, Self::ObservingBaseline) => true,
            (Self::ObservingBaseline, Self::Planning) => true,
            (Self::Planning, Self::DryRun | Self::ApplyingCandidate | Self::ObservingBaseline) => {
                true
            }
            (Self::DryRun, Self::ApplyingCandidate | Self::ObservingBaseline) => true,
            (Self::ApplyingCandidate, Self::MeasuringCandidate) => true,
            (
                Self::MeasuringCandidate,
                Self::KeepingCandidate | Self::RevertingCandidate | Self::ObservingBaseline,
            ) => true,
            (Self::KeepingCandidate | Self::RevertingCandidate, Self::ObservingBaseline) => true,
            (Self::Faulted, _) => false,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutotuneRuntimeTransitionError {
    pub(crate) from: AutotuneRuntimePhase,
    pub(crate) to: AutotuneRuntimePhase,
    pub(crate) reason: String,
}

impl fmt::Display for AutotuneRuntimeTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid autotune runtime phase transition {:?}->{:?}: {}",
            self.from, self.to, self.reason
        )
    }
}

impl AutotuneRuntime {
    pub(crate) fn runtime_phase(&self) -> AutotuneRuntimePhase {
        self.phase_machine.phase()
    }

    pub(super) fn transition_runtime_phase(
        &mut self,
        next: AutotuneRuntimePhase,
        reason: impl Into<String>,
    ) -> anyhow::Result<()> {
        if let Err(err) = self.phase_machine.transition(next, reason) {
            let message = err.to_string();
            log::error!("{message}");
            self.phase_machine.force_fault(message.clone());
            self.controller.state.phase = ControllerPhase::Faulted;
            anyhow::bail!(message);
        }
        Ok(())
    }

    pub(super) fn fault_runtime_phase(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        log::error!("{reason}");
        self.phase_machine.force_fault(reason);
        self.controller.state.phase = ControllerPhase::Faulted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::runtime::AutotuneRuntimeConfig;

    #[test]
    fn runtime_phase_transition_table_documents_allowed_and_denied_edges() {
        let cases = [
            (
                AutotuneRuntimePhase::Idle,
                AutotuneRuntimePhase::ObservingBaseline,
                true,
            ),
            (
                AutotuneRuntimePhase::MeasuringCandidate,
                AutotuneRuntimePhase::KeepingCandidate,
                true,
            ),
            (
                AutotuneRuntimePhase::MeasuringCandidate,
                AutotuneRuntimePhase::ApplyingCandidate,
                false,
            ),
            (
                AutotuneRuntimePhase::Faulted,
                AutotuneRuntimePhase::ApplyingCandidate,
                false,
            ),
        ];

        for (from, to, allowed) in cases {
            assert_eq!(
                from.allows_transition_to(to),
                allowed,
                "transition {from:?}->{to:?}"
            );
        }
    }

    #[test]
    fn denied_runtime_transition_forces_faulted_state() {
        let mut runtime =
            AutotuneRuntime::new(AutotuneRuntimeConfig::observe(None, Some(1234), None));
        runtime
            .transition_runtime_phase(AutotuneRuntimePhase::ObservingBaseline, "test observation")
            .unwrap();
        runtime
            .transition_runtime_phase(AutotuneRuntimePhase::Planning, "test planning")
            .unwrap();
        runtime
            .transition_runtime_phase(AutotuneRuntimePhase::ApplyingCandidate, "test apply")
            .unwrap();
        runtime
            .transition_runtime_phase(AutotuneRuntimePhase::MeasuringCandidate, "test measure")
            .unwrap();

        let err = runtime
            .transition_runtime_phase(
                AutotuneRuntimePhase::ApplyingCandidate,
                "candidate cannot be applied while measurement is active",
            )
            .unwrap_err();

        assert!(err.to_string().contains("MeasuringCandidate"));
        assert_eq!(runtime.runtime_phase(), AutotuneRuntimePhase::Faulted);
        assert_eq!(runtime.controller.state.phase, ControllerPhase::Faulted);
    }
}
