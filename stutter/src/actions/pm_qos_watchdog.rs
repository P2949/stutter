//! An independently enforced hold deadline for a PM QoS lease (package W11d).
//!
//! `PmQosHeld::expired` only reports; here a dedicated thread owns the lease and releases it at
//! the policy's `max_hold` whether or not the owner ever calls back. The owner may release early.
//! If the holder process dies the kernel closes the descriptor, so there is no state left behind.
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use super::pm_qos_transaction::{PmQosHeld, ReleaseOutcome};

#[derive(Debug, PartialEq, Eq)]
pub struct WatchdogResult {
    pub outcome: ReleaseOutcome,
    /// True when the deadline, not the owner, ended the lease.
    pub ended_by_deadline: bool,
}

type Signal = Arc<(Mutex<bool>, Condvar)>;

pub struct PmQosWatchdog {
    signal: Signal,
    handle: Option<JoinHandle<WatchdogResult>>,
}

/// Hands the lease to a watchdog that releases it no later than `held`'s hold limit.
pub fn enforce_deadline(held: PmQosHeld) -> std::io::Result<PmQosWatchdog> {
    enforce_deadline_within(held, None)
}

/// As `enforce_deadline`, optionally with a shorter limit chosen by the owner.
pub fn enforce_deadline_within(
    held: PmQosHeld,
    shorter: Option<Duration>,
) -> std::io::Result<PmQosWatchdog> {
    let limit = shorter.map_or(held.max_hold(), |shorter| shorter.min(held.max_hold()));
    let signal: Signal = Arc::new((Mutex::new(false), Condvar::new()));
    let thread_signal = signal.clone();
    let handle = std::thread::Builder::new()
        .name("pm-qos-watchdog".into())
        .spawn(move || {
            let deadline = Instant::now() + limit;
            let (lock, wake) = &*thread_signal;
            let mut released = lock.lock().unwrap_or_else(|poison| poison.into_inner());
            while !*released {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                released = wake
                    .wait_timeout(released, deadline - now)
                    .unwrap_or_else(|poison| poison.into_inner())
                    .0;
            }
            let ended_by_deadline = !*released;
            drop(released);
            WatchdogResult {
                outcome: held.restore(),
                ended_by_deadline,
            }
        })?;
    Ok(PmQosWatchdog {
        signal,
        handle: Some(handle),
    })
}

impl PmQosWatchdog {
    fn wake(&self) {
        let (lock, wake) = &*self.signal;
        *lock.lock().unwrap_or_else(|poison| poison.into_inner()) = true;
        wake.notify_all();
    }

    /// Releases now (if the deadline has not already) and reports how the lease ended.
    pub fn release(mut self) -> WatchdogResult {
        self.wake();
        self.join()
    }

    fn join(&mut self) -> WatchdogResult {
        match self.handle.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            // A panicked watchdog dropped the lease while unwinding: released, but unproven.
            _ => WatchdogResult {
                outcome: ReleaseOutcome::Unproven,
                ended_by_deadline: false,
            },
        }
    }

    /// Whether the lease has already ended.
    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for PmQosWatchdog {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.wake();
            let _ = self.join();
        }
    }
}

#[cfg(test)]
#[path = "pm_qos_watchdog_tests.rs"]
mod tests;
