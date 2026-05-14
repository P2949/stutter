use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonCapabilities {
    pub kernel_release: Option<String>,
    pub btf_available: bool,
    pub sched_tracepoints_available: bool,
    pub perf_permissions_likely: bool,
    pub perf_event_paranoid: Option<i32>,
    pub cgroup_v2_available: bool,
    pub sched_ext_available: bool,
    pub uclamp_available: bool,
    pub ionice_available: bool,
    pub irq_affinity_available: bool,
    pub gpu_sysfs_available: bool,
}

impl DaemonCapabilities {
    pub fn unavailable_features(&self) -> Vec<&'static str> {
        let mut features = Vec::new();

        if !self.btf_available {
            features.push("btf");
        }
        if !self.sched_tracepoints_available {
            features.push("sched_tracepoints");
        }
        if !self.perf_permissions_likely {
            features.push("perf_permissions");
        }
        if !self.cgroup_v2_available {
            features.push("cgroup_v2");
        }
        if !self.sched_ext_available {
            features.push("sched_ext");
        }
        if !self.uclamp_available {
            features.push("uclamp");
        }
        if !self.ionice_available {
            features.push("ionice");
        }
        if !self.irq_affinity_available {
            features.push("irq_affinity");
        }
        if !self.gpu_sysfs_available {
            features.push("gpu_sysfs");
        }

        features
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityProbeRoot {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
}

impl Default for CapabilityProbeRoot {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            sys_root: PathBuf::from("/sys"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityProbe {
    root: CapabilityProbeRoot,
}

impl CapabilityProbe {
    pub fn new(root: CapabilityProbeRoot) -> Self {
        Self { root }
    }

    pub fn probe(&self) -> DaemonCapabilities {
        let kernel_release = read_trimmed(&self.proc_path("sys/kernel/osrelease"));
        let perf_event_paranoid = read_trimmed(&self.proc_path("sys/kernel/perf_event_paranoid"))
            .and_then(|value| value.parse::<i32>().ok());

        DaemonCapabilities {
            kernel_release,
            btf_available: self.sys_path("kernel/btf/vmlinux").is_file(),
            sched_tracepoints_available: self.sched_tracepoints_available(),
            perf_permissions_likely: perf_event_paranoid.is_none_or(|value| value <= 2),
            perf_event_paranoid,
            cgroup_v2_available: self.sys_path("fs/cgroup/cgroup.controllers").is_file(),
            sched_ext_available: self.sys_path("kernel/sched_ext").exists(),
            uclamp_available: self.proc_path("sys/kernel/sched_util_clamp_min").is_file()
                || self.proc_path("sys/kernel/sched_util_clamp_max").is_file(),
            ionice_available: cfg!(target_os = "linux"),
            irq_affinity_available: self.proc_path("irq/default_smp_affinity").is_file(),
            gpu_sysfs_available: self.sys_path("class/drm").is_dir(),
        }
    }

    fn sched_tracepoints_available(&self) -> bool {
        self.sys_path("kernel/tracing/events/sched/sched_switch")
            .exists()
            || self
                .sys_path("kernel/debug/tracing/events/sched/sched_switch")
                .exists()
    }

    fn proc_path(&self, suffix: &str) -> PathBuf {
        self.root.proc_root.join(suffix)
    }

    fn sys_path(&self, suffix: &str) -> PathBuf {
        self.root.sys_root.join(suffix)
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-capability-probe-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn capability_probe_detects_available_features_from_fake_roots() {
        let root = temp_root("available");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");

        write_file(&proc_root.join("sys/kernel/osrelease"), "6.9.1-test\n");
        write_file(&proc_root.join("sys/kernel/perf_event_paranoid"), "1\n");
        write_file(&proc_root.join("sys/kernel/sched_util_clamp_min"), "1024\n");
        write_file(&proc_root.join("irq/default_smp_affinity"), "ff\n");
        write_file(&sys_root.join("kernel/btf/vmlinux"), "btf");
        write_file(
            &sys_root.join("kernel/tracing/events/sched/sched_switch/id"),
            "1\n",
        );
        write_file(&sys_root.join("fs/cgroup/cgroup.controllers"), "cpu io\n");
        fs::create_dir_all(sys_root.join("kernel/sched_ext")).unwrap();
        fs::create_dir_all(sys_root.join("class/drm")).unwrap();

        let caps = CapabilityProbe::new(CapabilityProbeRoot {
            proc_root,
            sys_root,
        })
        .probe();

        assert_eq!(caps.kernel_release.as_deref(), Some("6.9.1-test"));
        assert!(caps.btf_available);
        assert!(caps.sched_tracepoints_available);
        assert!(caps.perf_permissions_likely);
        assert!(caps.cgroup_v2_available);
        assert!(caps.sched_ext_available);
        assert!(caps.uclamp_available);
        assert!(caps.irq_affinity_available);
        assert!(caps.gpu_sysfs_available);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn capability_probe_reports_missing_features() {
        let root = temp_root("missing");
        let caps = CapabilityProbe::new(CapabilityProbeRoot {
            proc_root: root.join("proc"),
            sys_root: root.join("sys"),
        })
        .probe();

        assert!(caps.unavailable_features().contains(&"btf"));
        assert!(caps.unavailable_features().contains(&"cgroup_v2"));
        assert!(caps.unavailable_features().contains(&"irq_affinity"));

        fs::remove_dir_all(root).ok();
    }
}
