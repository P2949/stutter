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
    pub use crate::integrations::{receipt::*, *};
}

pub mod sensors {
    //! Sensor descriptors, readings, the PSI sensor and the cost-bounded registry.
    pub use crate::autotune::sensors::*;
}

pub mod topology {
    //! Cache, die, capacity, NUMA-distance and PCI-locality facts with a change fingerprint.
    pub use crate::topology_v2::*;
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
            proc_view::live_process,
            quality::*,
            registration::*,
            session::*,
        },
    };
}

pub mod experiments {
    //! Immutable experiment designs, evidence and the interval statistics behind them.
    pub use crate::autotune::{experiment_design::*, statistics::*};
}
