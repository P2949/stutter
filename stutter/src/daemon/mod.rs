//! Daemon policy, state, privilege, health, and lifecycle facade.
//!
//! Owns:
//! - daemon state models, policy verdicts, capability and health probes, lifecycle decisions,
//!   watchdog/overhead/soak checks, state persistence, and privileged operation mediation.
//!
//! Does not own:
//! - CLI parsing, command dispatch, direct remote request handling, report rendering, or direct
//!   action mutation outside daemon policy and privilege boundaries.
//!
//! Allowed dependencies:
//! - actions, autotune coordination, config, process tree data, recorder/session models, and
//!   system probes needed to evaluate safe daemon behavior.
//!
//! Main entry points:
//! - `DaemonRuntime`, `DaemonPolicy`, `DaemonState`, `DaemonPhase`, and core daemon config
//!   types. Implementation-specific contracts must be imported from their owning child modules.
//!
//! Safety, mutation, and persistence invariants:
//! - every daemon-authorized mutation must pass through `DaemonPolicy` and privilege checks;
//! - persistent state must use the daemon state schema/versioned store types;
//! - health, watchdog, and lifecycle transitions must degrade or reject unsafe operations rather
//!   than silently continuing;
//! - this facade should keep top-level re-exports narrow and route subsystem contracts through
//!   owner modules or `api::daemon`.

pub(crate) mod acceptance;
pub(crate) mod autotune;
pub(crate) mod capabilities;
pub(crate) mod config;
pub(crate) mod explain;
pub(crate) mod health;
pub(crate) mod lifecycle;
pub(crate) mod monitor;
pub(crate) mod overhead;
pub(crate) mod policy;
pub(crate) mod privilege;
pub(crate) mod runtime;
pub(crate) mod soak;
pub(crate) mod state;
pub(crate) mod state_builders;
pub(crate) mod store;
pub(crate) mod testing;
pub(crate) mod watchdog;

pub(crate) use config::{CgroupTargetRole, DaemonCgroupTargetsConfig, DaemonConfig, DaemonPreset};
pub(crate) use policy::DaemonPolicy;
pub use policy::DaemonPolicyVerdict;
pub use runtime::{DaemonRuntime, DaemonRuntimeConfig, DaemonRuntimeEvent, DaemonTransition};
pub(crate) use state::{DaemonPhase, DaemonState};

#[cfg(test)]
mod tests {
    #[test]
    fn daemon_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "daemon child modules must stay crate-private and be exposed intentionally through api::daemon: {public_child_modules:?}"
        );
    }

    #[test]
    fn daemon_top_level_reexports_are_narrow() {
        let source = include_str!("mod.rs");

        let actual: Vec<&str> = source
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("pub(crate) use ") || line.starts_with("pub use "))
            .collect();
        let expected = vec![
            "pub(crate) use config::{CgroupTargetRole, DaemonCgroupTargetsConfig, DaemonConfig, DaemonPreset};",
            "pub(crate) use policy::DaemonPolicy;",
            "pub use policy::DaemonPolicyVerdict;",
            "pub use runtime::{DaemonRuntime, DaemonRuntimeConfig, DaemonRuntimeEvent, DaemonTransition};",
            "pub(crate) use state::{DaemonPhase, DaemonState};",
        ];

        assert_eq!(
            actual, expected,
            "daemon top-level reexports must stay limited to the narrow runtime, policy, state, and core config facade"
        );
    }
}
