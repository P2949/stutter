//! Daemon acceptance, soak, and fake simulation helper façade.
//!
//! These helpers are intentionally kept out of the daemon root façade and out of
//! the public `api::daemon` production contract.

pub use crate::daemon::{
    acceptance::{DaemonAcceptanceReport, run_fake_daemon_acceptance_suite},
    soak::{
        DaemonSoakBudget, DaemonSoakConfig, DaemonSoakProfile, DaemonSoakReport,
        run_fake_daemon_soak,
    },
};
#[cfg(test)]
pub use crate::{
    autotune::simulation::{
        FakeDaemonScenario, FakeDaemonSimulationReport, FakeDaemonStep,
        assert_no_apply_without_rollback, run_fake_daemon_scenario,
        run_fake_daemon_scenario_with_safety,
    },
    daemon::{
        acceptance::DaemonAcceptanceStep,
        soak::{
            DaemonSoakFailure, DaemonSoakMetrics, DaemonSoakScenarioReport, SoakAssertion,
            SoakScenario, SoakTick, run_scenario_daemon_soak,
        },
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_testing_facade_exposes_acceptance_soak_and_simulation_helpers() {
        let _ = run_fake_daemon_acceptance_suite();
        let config = DaemonSoakConfig::default();
        let _ = run_fake_daemon_soak(&config);

        let _ = std::mem::size_of::<FakeDaemonScenario>();
        let _ = std::mem::size_of::<FakeDaemonSimulationReport>();
        let _ = std::mem::size_of::<FakeDaemonStep>();
        let _ = std::mem::size_of::<DaemonAcceptanceStep>();
        let _ = std::mem::size_of::<DaemonSoakFailure>();
        let _ = std::mem::size_of::<DaemonSoakMetrics>();
        let _ = std::mem::size_of::<DaemonSoakScenarioReport>();
        let _ = std::mem::size_of::<SoakAssertion>();
        let _ = std::mem::size_of::<SoakScenario>();
        let _ = std::mem::size_of::<SoakTick>();

        let _ = run_fake_daemon_scenario as *const ();
        let _ = run_fake_daemon_scenario_with_safety as *const ();
        let _ = run_scenario_daemon_soak as *const ();
        let _ = assert_no_apply_without_rollback as *const ();
    }
}
