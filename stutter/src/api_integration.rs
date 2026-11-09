//! Integration-era public modules, re-exported by `api` so the façade file stays small.

pub mod game_run {
    //! Launch records written by `stutter game-run` for campaigns that consume its capture directories.
    pub use crate::game_run::{LaunchRecord, default_capture_root, read_launch_record};
}

pub mod objective_source {
    //! Typed local application KPIs: contract, bound-source ingestion and window aggregation.
    pub use crate::autotune::objective::source::*;
}

pub mod ownership {
    //! Exact resource ownership receipts and the process-name owner probe.
    pub use crate::{
        actions::precommit::owner_unchanged,
        integrations::{gpu_identity::*, receipt::*, *},
        irq_attribution::*,
    };
}

pub mod sensors {
    //! Sensor descriptors, readings, the PSI sensor and the cost-bounded registry.
    pub use crate::autotune::sensors::*;
}

pub mod topology {
    //! Cache, die, capacity, NUMA-distance and PCI-locality facts with a change fingerprint.
    pub use crate::{
        autotune::planning::profile_candidates::llc::*, topology_hwloc::*, topology_v2::*,
    };
}

pub mod telemetry {
    //! Workload identity, frame-source attribution, stream quality and confined capture files.
    pub use crate::{
        mangohud::{
            MangoHudTailSession, TailConfig, TailCounters, TailFailure, TailState, TailedFrame,
            drive_tail_session,
        },
        telemetry::{
            frame_source::*,
            gamescope::{GamescopeSupport, probe as probe_gamescope},
            identity::*,
            live_source::*,
            proc_view::{
                ClockNamespaceDenial, ClockNamespaceIdentity, live_process,
                shared_monotonic_namespace,
            },
            quality::*,
            registration::*,
            session::*,
        },
    };
}

pub mod experiments {
    //! Immutable experiment designs, evidence and the interval statistics behind them.
    pub use crate::autotune::{
        experiment_design::*,
        live_experiment::{KpiObservationStudyConfig, KpiObservationStudyEvidence},
        sched_compare::*,
        statistics::*,
    };
}

pub mod cgroup_control {
    //! Typed cgroup resource-control values and their read-only preflight.
    pub use crate::actions::cgroup::{
        control::*,
        control_transaction::{
            CommitError, ControlCommitted, ControlFiles, ControlPrepared, PrepareError, RealFiles,
            RestoreOutcome, prepare as prepare_control,
        },
    };
}

pub mod search {
    //! Offline candidate-search policies over a finite legal space, and their replay on history.
    pub use crate::autotune::search::*;
}

pub mod boot_environment {
    //! The boot environment's identity: kernel, preemption model, isolation parameters, generation.
    pub use crate::{boot_campaign::*, boot_environment::*};
}

pub mod pm_qos {
    //! The PM QoS CPU-latency lease: policy, power guards and a descriptor-bound request.
    pub use crate::actions::{pm_qos::*, pm_qos_transaction::*, pm_qos_watchdog::*};
}

pub mod owner_interfaces {
    //! The read-only D-Bus client, exact owner states, and the gated offline `scx_loader` and
    //! power-profiles-daemon adapters.
    pub use crate::integrations::{
        dbus::*,
        exact::*,
        power_profiles::{
            Hold, HoldDenial, HoldError, HoldPlan, PROFILES as POWER_PROFILES,
            acquire as acquire_profile_hold, plan_hold as plan_profile_hold,
            read_active as read_active_power_profile,
        },
        scx_loader::*,
        user_intent::*,
    };
}

pub mod resctrl_allocation {
    //! Experimental resctrl L3 allocation as a prepared transaction (fake-root verified).
    pub use crate::actions::resctrl_alloc::{
        CatCommitError, CatCommitted, CatDenial, CatPrepared, CatRequest, CatRestore,
        prepare as prepare_cat,
    };
}
