//! The bounded CPU load the owned-pressure control applies (PLAN G5). It is the campaign's own: started by the
//! campaign inside its own process, bounded in size and in time, stopped by the campaign (or by its own deadline if the
//! campaign never does), and joined, so the record says the load ended and every thread of it was accounted for. It never
//! touches another process, a device or a file. It is a fixture workload: it exists only so the controller can be shown to
//! act when pressure does exist, and nothing here decides whether it should run.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// The most worker threads a control may run: a load is meant to press the scheduler, never to starve the machine.
pub const MAX_WORKERS: usize = 4;
/// The longest a control's load may run before its own deadline stops it.
pub const MAX_SECONDS: u64 = 300;

/// What a finished load says of itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRecord {
    /// The process the load ran in (the campaign's own) and its start time.
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
    pub workers: u32,
    pub started_unix: u64,
    pub stopped_unix: u64,
    /// Every worker thread was joined: the load is over, not merely asked to stop.
    pub all_joined: bool,
}

/// A running load. Dropping it stops and joins it.
pub struct OwnedLoad {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    spins: Arc<AtomicU64>,
    /// Workers still running, so a test (or the campaign) can see the load really ended.
    alive: Arc<AtomicUsize>,
    record: LoadRecord,
}

impl OwnedLoad {
    /// Start `workers` busy threads that stop at `max_seconds` on their own. Refused when the load would be larger than
    /// the machine can spare (at most `MAX_WORKERS`, and always at least one CPU left free) or longer than `MAX_SECONDS`.
    pub fn start(workers: usize, max_seconds: u64, now_unix: u64) -> Result<Self, &'static str> {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        if workers == 0 || workers > MAX_WORKERS || workers >= cpus {
            return Err("OwnedLoadSizeRefused");
        }
        if max_seconds == 0 || max_seconds > MAX_SECONDS {
            return Err("OwnedLoadDurationRefused");
        }
        let pid = std::process::id();
        let start_ticks =
            crate::process_tree::process_starttime_at(std::path::Path::new("/proc"), pid)
                .ok_or("OwnedLoadIdentityUnknown")?;
        let stop = Arc::new(AtomicBool::new(false));
        let spins = Arc::new(AtomicU64::new(0));
        let alive = Arc::new(AtomicUsize::new(workers));
        let deadline = Instant::now() + Duration::from_secs(max_seconds);
        let threads = (0..workers)
            .map(|_| {
                let (stop, spins, alive) = (stop.clone(), spins.clone(), alive.clone());
                std::thread::spawn(move || {
                    let mut local = 0u64;
                    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                        for _ in 0..10_000 {
                            local = local.wrapping_add(1);
                            std::hint::black_box(local);
                        }
                    }
                    spins.fetch_add(local, Ordering::Relaxed);
                    alive.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        Ok(Self {
            stop,
            threads,
            spins,
            alive,
            record: LoadRecord {
                pid: stutter_core::ids::Pid::new(pid),
                start_ticks,
                uid: crate::syscall::geteuid(),
                workers: u32::try_from(workers).map_err(|_| "OwnedLoadSizeRefused")?,
                started_unix: now_unix,
                stopped_unix: 0,
                all_joined: false,
            },
        })
    }

    /// Stop the load and join every thread, and say so.
    pub fn stop(mut self, now_unix: u64) -> LoadRecord {
        self.finish(now_unix);
        self.record.clone()
    }

    /// How many workers are still running.
    pub fn alive(&self) -> usize {
        self.alive.load(Ordering::SeqCst)
    }

    /// A handle that keeps counting the running workers after the load itself is gone.
    pub fn alive_handle(&self) -> Arc<AtomicUsize> {
        self.alive.clone()
    }

    /// Iterations the workers completed (a load that did nothing is not a load).
    pub fn spins(&self) -> u64 {
        self.spins.load(Ordering::Relaxed)
    }

    fn finish(&mut self, now_unix: u64) {
        self.stop.store(true, Ordering::Relaxed);
        let mut joined = true;
        for thread in self.threads.drain(..) {
            joined &= thread.join().is_ok();
        }
        self.record.stopped_unix = now_unix.max(self.record.started_unix);
        self.record.all_joined = joined;
    }
}

impl Drop for OwnedLoad {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            self.finish(self.record.started_unix);
        }
    }
}

#[cfg(test)]
#[path = "owned_load_tests.rs"]
mod tests;
