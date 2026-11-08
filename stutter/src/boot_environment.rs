//! The identity of the boot environment (package W14): kernel, preemption model, isolation
//! parameters, microcode, scheduler, generation and a hash of all of it.
//!
//! Boot settings are not live tunables. This only *describes* the environment so that a result is
//! never carried across a different kernel, a different `nohz_full`/`isolcpus` set or a different
//! generation. The raw command line is never stored (it can hold device identifiers); only the
//! parsed isolation keys and a hash of the whole line are.
use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    daemon::state::daemon_profile_stable_hash,
    topology::{parse_cpu_list, sorted_unique},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreemptMode {
    /// `PREEMPT_RT`: the real-time kernel.
    Rt,
    /// Selectable at boot (`PREEMPT_DYNAMIC`); the mode in force is not read here.
    Dynamic,
    /// `PREEMPT` without dynamic selection.
    Preempt,
    /// Neither marker: voluntary or no forced preemption.
    NotPreemptible,
}

/// The CPU-isolation parameters on the command line, as CPU lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Isolation {
    pub nohz_full: Vec<u32>,
    pub rcu_nocbs: Vec<u32>,
    /// `isolcpus=` CPUs, with any flags ignored.
    pub isolcpus: Vec<u32>,
    pub irqaffinity: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootEnvironment {
    pub boot_id: String,
    pub kernel_release: String,
    pub preempt: PreemptMode,
    pub isolation: Isolation,
    pub microcode: Option<String>,
    /// The base name of the active system generation (for example a Nix store path), if known.
    pub generation: Option<String>,
    /// `sched_ext` scheduler loaded, if any.
    pub scheduler_ext_ops: Option<String>,
    /// Hash of the whole command line, so a changed parameter changes the identity.
    pub cmdline_hash: String,
    pub environment_hash: String,
}

pub fn preempt_mode(kernel_version: &str) -> PreemptMode {
    if kernel_version.contains("PREEMPT_RT") {
        PreemptMode::Rt
    } else if kernel_version.contains("PREEMPT_DYNAMIC") {
        PreemptMode::Dynamic
    } else if kernel_version
        .split_whitespace()
        .any(|word| word == "PREEMPT")
    {
        PreemptMode::Preempt
    } else {
        PreemptMode::NotPreemptible
    }
}

/// CPU list of `key=` on the command line. `isolcpus` may begin with flags (`domain,managed_irq,`).
fn cpu_param(cmdline: &str, key: &str) -> Vec<u32> {
    let needle = format!("{key}=");
    cmdline
        .split_whitespace()
        .find_map(|word| word.strip_prefix(needle.as_str()))
        .map(|value| {
            // Drop leading flag words, keep the list.
            let list = value
                .split(',')
                .skip_while(|part| part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
                .collect::<Vec<_>>()
                .join(",");
            parse_cpu_list(&list).map(sorted_unique).unwrap_or_default()
        })
        .unwrap_or_default()
}

pub fn parse_isolation(cmdline: &str) -> Isolation {
    Isolation {
        nohz_full: cpu_param(cmdline, "nohz_full"),
        rcu_nocbs: cpu_param(cmdline, "rcu_nocbs"),
        isolcpus: cpu_param(cmdline, "isolcpus"),
        irqaffinity: cpu_param(cmdline, "irqaffinity"),
    }
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

impl BootEnvironment {
    /// Reads the environment from `proc_root`, `sys_root` and the `current-system` link under
    /// `run_root`.
    pub fn read_at(proc_root: &Path, sys_root: &Path, run_root: &Path) -> Self {
        let cmdline = read(&proc_root.join("cmdline")).unwrap_or_default();
        let version = read(&proc_root.join("sys/kernel/version")).unwrap_or_default();
        let microcode = read(&proc_root.join("cpuinfo")).and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("microcode"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        });
        let generation = fs::read_link(run_root.join("current-system"))
            .ok()
            .and_then(|target| {
                target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let scheduler_ext_ops = match read(&sys_root.join("kernel/sched_ext/state")).as_deref() {
            Some("enabled") => read(&sys_root.join("kernel/sched_ext/root/ops")),
            _ => None,
        };
        let mut environment = Self {
            boot_id: read(&proc_root.join("sys/kernel/random/boot_id")).unwrap_or_default(),
            kernel_release: read(&proc_root.join("sys/kernel/osrelease")).unwrap_or_default(),
            preempt: preempt_mode(&version),
            isolation: parse_isolation(&cmdline),
            microcode,
            generation,
            scheduler_ext_ops,
            cmdline_hash: daemon_profile_stable_hash([cmdline.as_str()]),
            environment_hash: String::new(),
        };
        // The boot id is excluded: the same environment after a reboot is the same environment.
        let rendered = format!(
            "{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}",
            environment.kernel_release,
            environment.preempt,
            environment.isolation,
            environment.microcode,
            environment.generation,
            environment.scheduler_ext_ops,
            environment.cmdline_hash
        );
        environment.environment_hash = daemon_profile_stable_hash([rendered.as_str()]);
        environment
    }

    /// CPUs left for general work: the online CPUs minus every isolated set.
    pub fn housekeeping_cpus(&self, online: &[u32]) -> Vec<u32> {
        let isolated: std::collections::BTreeSet<u32> = self
            .isolation
            .nohz_full
            .iter()
            .chain(&self.isolation.rcu_nocbs)
            .chain(&self.isolation.isolcpus)
            .copied()
            .collect();
        online
            .iter()
            .copied()
            .filter(|cpu| !isolated.contains(cpu))
            .collect()
    }

    /// Whether a result recorded under `hash` still applies to this environment.
    pub fn same_environment(&self, hash: &str) -> bool {
        self.environment_hash == hash
    }
}

#[cfg(test)]
#[path = "boot_environment_tests.rs"]
mod tests;
