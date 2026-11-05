//! The hard wall-time budget of a counted service soak, and the dwell phase that finishes it.
//! Neither adds nor removes coverage: a round that cannot finish inside the budget is not started,
//! and a coverage-complete run keeps the real packaged service alive and attested until the
//! independently verified dwell is reached.
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use anyhow::ensure;
use stutter_validation_runner::model::BoundedServiceAction as Action;

use super::{
    plan::{Kind, Round, Timing},
    records::{SoakState, unix_ms},
    round::STOP,
    service_presence, world,
};

/// Time kept back for the service to stop and its evidence to settle after the last round.
pub(super) const SETTLEMENT_MS: u64 = 300_000;
const ATTEST_EVERY: Duration = Duration::from_secs(60);

/// The longest the round may run. A long round keeps its workload for `long_round_s` and then
/// waits for the experiment to resolve; every other round waits for up to two resolutions.
pub(super) fn round_reserve_ms(round: &Round, timing: Timing) -> u64 {
    let seconds = if round.kind == Kind::Long {
        timing.long_round_s + timing.resolve_timeout_s + 120
    } else {
        timing.resolve_timeout_s * 2 + 120
    };
    seconds * 1000 + SETTLEMENT_MS
}

/// Whether a round with this reserve, starting at `now`, still finishes inside the cap.
pub(super) fn fits(now_ms: u64, cap_ms: u64, reserve_ms: u64) -> bool {
    now_ms.saturating_add(reserve_ms) <= cap_ms
}

/// Keeps the service live under normal observation until the verified dwell is reached, the cap
/// passes or the soak is interrupted. The independent presence collector alone accumulates the
/// dwell; this loop only proves, every minute, that the same pair is still intact and settled.
pub(super) fn idle_dwell(
    dir: &std::path::Path,
    state: &SoakState,
    world: &world::World,
    cap_ms: u64,
    needed_ms: u64,
) -> anyhow::Result<()> {
    let binding = state
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing service binding"))?;
    let service = world
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing service binding"))?;
    let mut identity: Option<(u64, u64, u64, u64)> = None;
    let mut next = Instant::now();
    while !STOP.load(Ordering::SeqCst) && unix_ms() < cap_ms {
        if Instant::now() >= next {
            let report = service.action(Action::Query)?;
            super::service::attest(&report, &binding.campaign)?;
            ensure!(
                report["worker_tokens"]["readable"] == true
                    && report["worker_tokens"]["outstanding"] == 0,
                "the idle service holds an unsettled token: {}",
                report["worker_tokens"]
            );
            let seen = (
                report["controller_process"]["pid"].as_u64().unwrap_or(0),
                report["controller_process"]["starttime_ticks"]
                    .as_u64()
                    .unwrap_or(0),
                report["worker_process"]["pid"].as_u64().unwrap_or(0),
                report["worker_process"]["starttime_ticks"]
                    .as_u64()
                    .unwrap_or(0),
            );
            ensure!(
                *identity.get_or_insert(seen) == seen,
                "the service pair changed during the idle dwell"
            );
            next = Instant::now() + ATTEST_EVERY;
        }
        if service_presence::verified_ms(dir, &binding.campaign)? >= needed_ms {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

/// A rehearsal runs the same phase for two minutes so its first live use is never the counted run.
pub(super) fn rehearse_idle_dwell(
    dir: &std::path::Path,
    state: &SoakState,
    world: &world::World,
    cap_ms: u64,
) -> anyhow::Result<()> {
    let binding = state
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing service binding"))?;
    let earned = service_presence::verified_ms(dir, &binding.campaign)?;
    idle_dwell(dir, state, world, cap_ms, earned + 120_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soak::plan;

    #[test]
    fn a_round_that_cannot_finish_inside_the_cap_is_not_started() {
        assert!(fits(0, 1_000, 1_000));
        assert!(!fits(1, 1_000, 1_000));
        assert!(!fits(u64::MAX, u64::MAX - 1, 5));
    }

    #[test]
    fn the_long_round_reserves_more_than_any_other() {
        let timing = plan::timing(crate::soak::records::Profile::Service);
        let long = plan::service_round_for((plan::SERVICE_CYCLE_LEN - 1) as u32);
        assert_eq!(long.kind, Kind::Long);
        let plain = plan::service_round_for(0);
        assert!(round_reserve_ms(&long, timing) > round_reserve_ms(&plain, timing));
        assert!(round_reserve_ms(&long, timing) >= (timing.long_round_s + 120) * 1000);
    }
}
