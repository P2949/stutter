//! Host facts the crosswalk observes, read from the kernel's own files while an attempt runs (never assumed). Each fact has
//! a counter name the resident window carries and a rule for what counts as one look that showed it:
//!
//! * `topology_samples` (5.9): the online CPU list and a CPU's thread siblings are readable;
//! * `single_numa_samples` (5.19): the online node list is exactly one node, so no inter-node migration is possible;
//! * `memory_guard_samples` (5.20): memory PSI, swap and the OOM-kill counter are all readable;
//! * `irq_placement_samples` (5.18): the interrupt table and the default affinity mask are readable;
//! * `power_policy_samples` (5.16): a CPU's scaling governor and energy-performance preference are readable;
//! * `sched_ext_samples` (5.6): the sched_ext state file names a state;
//! * `damon_samples` (5.8): the DAMON admin interface reports its kdamond count;
//! * `resctrl_absent_samples` (5.24): the CPU flags are readable and show no RDT allocation;
//! * `io_pressure_samples` (5.26): the IO and CPU pressure files are readable.
//! * `boot_cmdline_samples` (extra.boot): the kernel command line (where nohz_full, isolcpus and rcu_nocbs would show) is readable;
//!
//! The root is a parameter so the files can be a fixture's; production reads `/`. A file that is absent, unreadable or not
//! in the expected shape counts for nothing.

use std::{io::Read, path::Path};

const MAX_BYTES: u64 = 64 * 1024;

/// What one look at the host showed: for each counter, whether this look counts.
pub type HostReadings = Vec<(&'static str, bool)>;

fn read(root: &Path, relative: &str) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(root.join(relative))
        .ok()?
        .take(MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn nonempty(root: &Path, relative: &str) -> bool {
    read(root, relative).is_some_and(|text| !text.trim().is_empty())
}

fn pressure(root: &Path, relative: &str) -> bool {
    read(root, relative).is_some_and(|text| text.starts_with("some "))
}

/// Look at the host's files under `root`.
pub fn readings(root: &Path) -> HostReadings {
    let cpu = "sys/devices/system/cpu";
    let flags = read(root, "proc/cpuinfo").and_then(|text| {
        text.lines()
            .find(|line| line.starts_with("flags"))
            .map(str::to_owned)
    });
    vec![
        (
            "topology_samples",
            nonempty(root, &format!("{cpu}/online"))
                && nonempty(root, &format!("{cpu}/cpu0/topology/thread_siblings_list")),
        ),
        (
            "single_numa_samples",
            read(root, "sys/devices/system/node/online").is_some_and(|text| text.trim() == "0"),
        ),
        (
            "memory_guard_samples",
            pressure(root, "proc/pressure/memory")
                && read(root, "proc/swaps").is_some()
                && read(root, "proc/vmstat")
                    .is_some_and(|text| text.lines().any(|line| line.starts_with("oom_kill "))),
        ),
        (
            "irq_placement_samples",
            read(root, "proc/interrupts").is_some_and(|text| text.lines().count() > 1)
                && nonempty(root, "proc/irq/default_smp_affinity"),
        ),
        (
            "power_policy_samples",
            nonempty(root, &format!("{cpu}/cpu0/cpufreq/scaling_governor"))
                && nonempty(
                    root,
                    &format!("{cpu}/cpu0/cpufreq/energy_performance_preference"),
                ),
        ),
        (
            "sched_ext_samples",
            nonempty(root, "sys/kernel/sched_ext/state"),
        ),
        (
            "damon_samples",
            read(root, "sys/kernel/mm/damon/admin/kdamonds/nr_kdamonds")
                .is_some_and(|text| text.trim().parse::<u32>().is_ok()),
        ),
        (
            "resctrl_absent_samples",
            flags.is_some_and(|line| !line.split_whitespace().any(|flag| flag == "rdt_a"))
                && !root.join("sys/fs/resctrl/schemata").exists(),
        ),
        ("boot_cmdline_samples", nonempty(root, "proc/cmdline")),
        (
            "io_pressure_samples",
            pressure(root, "proc/pressure/io") && pressure(root, "proc/pressure/cpu"),
        ),
    ]
}

#[cfg(test)]
#[path = "host_probe_tests.rs"]
mod tests;
