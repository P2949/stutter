//! Owner probing and the safety stops that restore a running experiment: the cached owner probe,
//! the memory and thermal session guards, and an owner that newly claims the experiment's domain.
use super::AutotuneRuntime;

/// How long an owner probe stays valid before `/proc` is scanned again.
const OWNER_PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

impl AutotuneRuntime {
    /// The other tuning owners on the host, probed at most once per `OWNER_PROBE_TTL`.
    pub(super) fn current_external_owners(&self) -> Option<crate::integrations::OwnerReport> {
        let mut cache = self.owner_cache.lock().ok()?;
        let now = std::time::Instant::now();
        if let Some((at, report)) = cache.as_ref()
            && now.duration_since(*at) < OWNER_PROBE_TTL
        {
            return Some(report.clone());
        }
        let mut report =
            crate::integrations::probe(std::path::Path::new("/proc"), std::path::Path::new("/sys"));
        // GameMode only acts for registered clients: its own interface can say it serves nobody.
        if report.daemon(crate::integrations::KnownOwner::GameMode)
            == crate::integrations::Daemon::Running
        {
            let (clients, _games) = crate::integrations::exact::read_gamemode();
            crate::integrations::exact::refine_gamemode(&mut report, &clients);
        }
        // TuneD only owns what its active profile manages: read the profile to say which.
        if report.daemon(crate::integrations::KnownOwner::Tuned)
            == crate::integrations::Daemon::Running
        {
            crate::integrations::tuned::refine_tuned(
                &mut report,
                crate::integrations::tuned::current_managed_attributes().as_ref(),
            );
        }
        // Tools with no long-running process (TLP) are visible only as an active systemd unit.
        crate::integrations::units::refine_with_units(
            &mut report,
            &crate::integrations::units::read_system_unit_states(),
        );
        *cache = Some((now, report.clone()));
        Some(report)
    }
}

impl AutotuneRuntime {
    /// Restores and invalidates a running experiment when the session's memory guard fired in
    /// `readings`. Returns whether it did. It never starts or changes anything.
    pub(super) fn abort_on_memory_guard(
        &mut self,
        readings: &[crate::autotune::sensors::SensorReading],
    ) -> anyhow::Result<bool> {
        let Some(event) = crate::autotune::sensors::memory_guard_event(readings)
            .or_else(|| crate::autotune::sensors::thermal_guard_event(readings))
            .or_else(|| crate::autotune::sensors::scheduler_change_event(readings))
        else {
            return Ok(false);
        };
        if !self.has_active_experiment() {
            return Ok(false);
        }
        self.rollback_live_experiment(
            crate::audit::unix_nanos_now(),
            &format!("session guard fired: {event}"),
        )?;
        Ok(true)
    }
}

impl AutotuneRuntime {
    /// Restores and invalidates a running experiment when another owner newly claimed the domain
    /// its operation changes, and remembers the current claims for the next probe.
    pub(super) fn abort_on_owner_appearance(
        &mut self,
        report: Option<&crate::integrations::OwnerReport>,
    ) -> anyhow::Result<bool> {
        let Some(report) = report else {
            return Ok(false);
        };
        let previous = std::mem::replace(
            &mut self.claimed_domains,
            crate::integrations::claimed_domains(report),
        );
        let Some(operation) = self
            .live_experiments
            .current_experiment()
            .map(|experiment| experiment.candidate.operation_kind())
        else {
            return Ok(false);
        };
        if !crate::integrations::owner_appeared_for(&previous, report, operation) {
            return Ok(false);
        }
        self.rollback_live_experiment(
            crate::audit::unix_nanos_now(),
            "another tuning owner claimed the domain this experiment changes",
        )?;
        Ok(true)
    }
}
