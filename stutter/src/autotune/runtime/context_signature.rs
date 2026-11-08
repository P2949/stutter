//! The context signature a stored experiment is keyed on.
//!
//! It joins the system inventory hash, the topology-v2 fingerprint (cache domains, capacity, NUMA,
//! GPU locality, the CPUs this process may use) and the boot environment's context hash (kernel,
//! preemption, isolation, microcode, scheduler). A hotplug, a cpuset change, a different kernel or a
//! new isolation setup therefore gives a different key, and an old result is not transferred. It is
//! recomputed at most once per `TTL`, never per tick.
use std::{
    path::Path,
    time::{Duration, Instant},
};

use super::AutotuneRuntime;
use crate::{
    boot_environment::BootEnvironment,
    topology::{TopologyModel, parse_cpu_list},
    topology_v2::TopologyV2,
};

const TTL: Duration = Duration::from_secs(60);

/// How the three parts are joined; a pure function so the format is fixed and testable.
pub(super) fn compose(
    inventory_hash: &str,
    topology_fingerprint: &str,
    context_hash: &str,
) -> String {
    format!("{inventory_hash}+{topology_fingerprint}+{context_hash}")
}

fn compute(inventory_hash: &str) -> String {
    let system = Path::new("/sys/devices/system");
    let Ok(base) = TopologyModel::read_at(system) else {
        // Without a topology the key stays what it was: an unknown topology is not guessed at.
        return inventory_hash.to_owned();
    };
    let effective = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .and_then(|list| parse_cpu_list(list.trim()).ok())
        })
        .unwrap_or_else(|| base.online_cpu_ids());
    let topology = TopologyV2::read_at(system, Path::new("/sys/class"), &base, effective);
    let environment =
        BootEnvironment::read_at(Path::new("/proc"), Path::new("/sys"), Path::new("/run"));
    compose(
        inventory_hash,
        &topology.fingerprint,
        &environment.context_hash,
    )
}

impl AutotuneRuntime {
    /// The signature for `inventory_hash`, cached for `TTL` so sysfs is not walked every tick.
    pub(super) fn context_signature(&self, inventory_hash: &str) -> String {
        let Ok(mut cache) = self.context_cache.lock() else {
            return inventory_hash.to_owned();
        };
        let now = Instant::now();
        if let Some((at, for_inventory, value)) = cache.as_ref()
            && now.duration_since(*at) < TTL
            && for_inventory == inventory_hash
        {
            return value.clone();
        }
        let value = compute(inventory_hash);
        *cache = Some((now, inventory_hash.to_owned(), value.clone()));
        value
    }
}
