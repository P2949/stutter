//! Out-of-memory guard state for the user's session: `memory.events` of the user slice.
//!
//! A kill, a throttle-at-`memory.high` or a hit of `memory.max` in the session is a reason to stop
//! trusting a comparison and never a reason to tune anything. The daemon's mere presence (oomd,
//! uresourced) does not prove it owns a cgroup; this reads the cgroup's own counters.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

/// The counters watched, in `memory.events`.
const COUNTERS: [&str; 5] = ["low", "high", "max", "oom", "oom_kill"];

/// The `user-<uid>.slice` cgroup directory for `uid`, from the cgroup file of a process in the
/// session (`0::/user.slice/user-1000.slice/...`). Falls back to the process's own cgroup when
/// the session is not under a user slice.
pub fn session_cgroup_dir(cgroup_root: &Path, proc_cgroup: &str, uid: u32) -> Option<PathBuf> {
    let path = proc_cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    let slice = format!("user-{uid}.slice");
    let relative: PathBuf = path
        .split('/')
        .filter(|part| !part.is_empty())
        .scan(false, |done, part| {
            if *done {
                return None;
            }
            *done = part == slice;
            Some(part)
        })
        .collect();
    (!relative.as_os_str().is_empty()).then(|| cgroup_root.join(relative))
}

fn read_events(dir: &Path) -> Option<BTreeMap<String, u64>> {
    let text = fs::read_to_string(dir.join("memory.events")).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                Some((key.to_owned(), value.trim().parse().ok()?))
            })
            .collect(),
    )
}

pub struct OomGuardSensor {
    dir: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl OomGuardSensor {
    pub fn new(cgroup_dir: PathBuf) -> Self {
        Self {
            dir: cgroup_dir,
            previous: None,
            health: SensorHealth::Healthy,
        }
    }
}

impl Sensor for OomGuardSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "oom_guard",
            cost: SensorCost::Cheap,
            scope: SensorScope::Cgroup,
            requires: "cgroup v2 memory.events of the user slice",
        }
    }

    fn probe(&self) -> Availability {
        if read_events(&self.dir).is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "{}/memory.events is not readable",
                self.dir.display()
            ))
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => {
                self.previous = None;
                Ok(())
            }
            Availability::Unavailable(reason) => {
                self.health = SensorHealth::Failed(reason.clone());
                Err(reason)
            }
        }
    }

    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        let mut values = BTreeMap::new();
        let mut lost = 0;
        let mut degraded = None;
        match read_events(&self.dir) {
            None => degraded = Some("memory.events became unreadable (cgroup removed?)".to_owned()),
            Some(now) => {
                if let Some(before) = &self.previous {
                    for key in COUNTERS {
                        let (Some(&after), Some(&was)) = (now.get(key), before.get(key)) else {
                            continue;
                        };
                        if after >= was {
                            values.insert(format!("{key}_delta"), (after - was) as f64);
                        } else {
                            lost += 1;
                            degraded = Some(format!("memory.events {key} went backwards"));
                        }
                    }
                }
                self.previous = Some(now);
            }
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        SensorReading {
            sensor: "oom_guard".to_owned(),
            monotonic_ns,
            scope: SensorScope::Cgroup,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: format!("{}/memory.events", self.dir.display()),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous = None;
    }
}
