//! Typed admission checks immediately before the authenticated Steam launch.
use super::{
    super::driver_failure::{FailureClass as Class, LaunchFailure},
    *,
};

impl<R: ResultProbe> ProductionSteamPort<R> {
    pub(super) fn launch_checked(
        &mut self,
        slot: &AttemptSlot,
        context: &DispatchContextV1,
    ) -> anyhow::Result<()> {
        LaunchFailure::require(
            self.transient.is_some(),
            Class::Environment,
            "BlockedTransientLauncherRequired",
        )?;
        LaunchFailure::require(
            !self.unsettled,
            Class::OwnershipUnresolved,
            "SettlementUnproven",
        )?;
        LaunchFailure::require(
            launcher_is_unprivileged(),
            Class::Integrity,
            "BlockedPrivilegedSteamLauncher",
        )?;
        LaunchFailure::require(
            launch_window_open(&context.intent, std::time::SystemTime::now()),
            Class::Resource,
            "AuthorizationExpiredBeforeLaunch",
        )?;
        LaunchFailure::require(
            self.idle_is_fresh() && self.steam_and_prefilter_clear(),
            Class::Environment,
            "SafeWindowClosed",
        )?;
        let dispatch = self
            .open_dispatch(slot, context)
            .map_err(|_| LaunchFailure {
                class: Class::Integrity,
                code: "DispatchIdentityUnverified",
            })?;
        if let Some(guard) = self.gpu.as_mut() {
            guard.begin_attempt();
        }
        self.probe.begin(slot.arm).map_err(|_| LaunchFailure {
            class: Class::Integrity,
            code: "NativeProbeAdmissionUnverified",
        })?;
        LaunchFailure::require(
            self.idle_is_fresh(),
            Class::Environment,
            "DesktopIdleExpired",
        )?;
        LaunchFailure::require(
            self.steam_session_paths_verified(),
            Class::Integrity,
            "BlockedSteamSessionEnvironment",
        )?;
        LaunchFailure::require(
            self.idle_is_fresh(),
            Class::Environment,
            "DesktopIdleExpired",
        )?;
        LaunchFailure::require(
            self.disk_reserve_ok(),
            Class::Resource,
            "BlockedDiskReserve",
        )?;
        LaunchFailure::require(
            self.gpu.as_mut().is_none_or(GpuGuard::admit_now),
            Class::Health,
            "BlockedGpuStability",
        )?;
        self.start_unit(dispatch, slot, context)
    }
}
