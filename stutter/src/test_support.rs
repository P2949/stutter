#![cfg(test)]

#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

pub struct TestRoot {
    path: PathBuf,
}

pub static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl TestRoot {
    pub fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "stutter-{prefix}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));

        fs::create_dir_all(&path)
            .unwrap_or_else(|err| panic!("failed to create test root {}: {err}", path.display()));

        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Debug)]
pub struct FakeThread {
    pub tid: u32,
    pub comm: String,
    pub starttime_ticks: u64,
}

impl FakeThread {
    pub fn new(tid: u32, comm: impl Into<String>, starttime_ticks: u64) -> Self {
        Self {
            tid,
            comm: comm.into(),
            starttime_ticks,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FakeProcess {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: Vec<String>,
    pub cgroup_path: String,
    pub starttime_ticks: u64,
    pub sched_policy: u32,
    pub threads: Vec<FakeThread>,
}

impl FakeProcess {
    pub fn new(pid: u32, comm: impl Into<String>, starttime_ticks: u64) -> Self {
        let comm = comm.into();
        Self {
            pid,
            ppid: 1,
            comm: comm.clone(),
            cmdline: vec![comm],
            cgroup_path: "/".to_owned(),
            starttime_ticks,
            sched_policy: 0,
            threads: Vec::new(),
        }
    }

    pub fn ppid(mut self, ppid: u32) -> Self {
        self.ppid = ppid;
        self
    }

    pub fn cgroup(mut self, cgroup_path: impl Into<String>) -> Self {
        self.cgroup_path = cgroup_path.into();
        self
    }

    pub fn cmdline(mut self, cmdline: Vec<String>) -> Self {
        self.cmdline = cmdline;
        self
    }

    pub fn sched_policy(mut self, sched_policy: u32) -> Self {
        self.sched_policy = sched_policy;
        self
    }

    pub fn thread(mut self, tid: u32, comm: impl Into<String>, starttime_ticks: u64) -> Self {
        self.threads
            .push(FakeThread::new(tid, comm, starttime_ticks));
        self
    }
}

pub struct FakeProc {
    root: TestRoot,
}

impl FakeProc {
    pub fn new(name: &str) -> Self {
        Self {
            root: TestRoot::new(&format!("fake-proc-{name}")),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn write_process(&self, process: FakeProcess) -> anyhow::Result<()> {
        let proc_dir = self.root.join(process.pid.to_string());
        fs::create_dir_all(proc_dir.join("task")).with_context(|| {
            format!(
                "failed to create fake proc task directory {}",
                proc_dir.display()
            )
        })?;

        fs::write(
            proc_dir.join("status"),
            format!("Name:\t{}\nPPid:\t{}\n", process.comm, process.ppid),
        )
        .with_context(|| format!("failed to write {}", proc_dir.join("status").display()))?;

        fs::write(proc_dir.join("cmdline"), process.cmdline.join("\0"))
            .with_context(|| format!("failed to write {}", proc_dir.join("cmdline").display()))?;

        fs::write(
            proc_dir.join("stat"),
            fake_stat_line(
                process.pid,
                &process.comm,
                process.ppid,
                process.starttime_ticks,
                process.sched_policy,
            ),
        )
        .with_context(|| format!("failed to write {}", proc_dir.join("stat").display()))?;

        fs::write(
            proc_dir.join("cgroup"),
            format!("0::{}\n", process.cgroup_path),
        )
        .with_context(|| format!("failed to write {}", proc_dir.join("cgroup").display()))?;

        let exe_target = self
            .root
            .join(format!("exe-target-{}-{}", process.pid, process.comm));
        fs::write(&exe_target, b"fake executable\n")
            .with_context(|| format!("failed to write {}", exe_target.display()))?;

        #[cfg(unix)]
        {
            let exe_link = proc_dir.join("exe");
            let _ = fs::remove_file(&exe_link);
            unix_fs::symlink(&exe_target, &exe_link).with_context(|| {
                format!(
                    "failed to symlink fake proc exe {} -> {}",
                    exe_link.display(),
                    exe_target.display()
                )
            })?;
        }

        let mut threads = process.threads;
        if !threads.iter().any(|thread| thread.tid == process.pid) {
            threads.push(FakeThread::new(
                process.pid,
                process.comm.clone(),
                process.starttime_ticks,
            ));
        }

        for thread in threads {
            let task_dir = proc_dir.join("task").join(thread.tid.to_string());
            fs::create_dir_all(&task_dir)
                .with_context(|| format!("failed to create {}", task_dir.display()))?;
            fs::write(task_dir.join("comm"), format!("{}\n", thread.comm))
                .with_context(|| format!("failed to write {}", task_dir.join("comm").display()))?;
            fs::write(
                task_dir.join("stat"),
                fake_stat_line(
                    thread.tid,
                    &thread.comm,
                    process.pid,
                    thread.starttime_ticks,
                    process.sched_policy,
                ),
            )
            .with_context(|| format!("failed to write {}", task_dir.join("stat").display()))?;
        }

        Ok(())
    }

    pub fn write_task_identity(
        &self,
        tid: u32,
        comm: &str,
        starttime_ticks: u64,
        cgroup_path: &str,
    ) -> anyhow::Result<()> {
        self.write_process(
            FakeProcess::new(tid, comm, starttime_ticks).cgroup(cgroup_path.to_owned()),
        )
    }
}

pub struct FakeSysfs {
    root: TestRoot,
}

impl FakeSysfs {
    pub fn new(name: &str) -> Self {
        Self {
            root: TestRoot::new(&format!("fake-sysfs-{name}")),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn write_cpu_cpufreq(
        &self,
        cpu: u32,
        scaling_governor: &str,
        energy_performance_preference: &str,
    ) -> anyhow::Result<PathBuf> {
        let cpufreq = self
            .root
            .join("devices/system/cpu")
            .join(format!("cpu{cpu}"))
            .join("cpufreq");

        fs::create_dir_all(&cpufreq)
            .with_context(|| format!("failed to create {}", cpufreq.display()))?;
        fs::write(
            cpufreq.join("scaling_governor"),
            format!("{scaling_governor}\n"),
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                cpufreq.join("scaling_governor").display()
            )
        })?;
        fs::write(
            cpufreq.join("energy_performance_preference"),
            format!("{energy_performance_preference}\n"),
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                cpufreq.join("energy_performance_preference").display()
            )
        })?;

        Ok(cpufreq)
    }

    pub fn write_ac_power(&self, online: bool) -> anyhow::Result<()> {
        let ac = self.root.join("class/power_supply/AC");
        fs::create_dir_all(&ac).with_context(|| format!("failed to create {}", ac.display()))?;
        fs::write(ac.join("type"), "Mains\n")
            .with_context(|| format!("failed to write {}", ac.join("type").display()))?;
        fs::write(ac.join("online"), if online { "1\n" } else { "0\n" })
            .with_context(|| format!("failed to write {}", ac.join("online").display()))?;
        Ok(())
    }

    pub fn write_battery(&self, status: &str) -> anyhow::Result<()> {
        let battery = self.root.join("class/power_supply/BAT0");
        fs::create_dir_all(&battery)
            .with_context(|| format!("failed to create {}", battery.display()))?;
        fs::write(battery.join("type"), "Battery\n")
            .with_context(|| format!("failed to write {}", battery.join("type").display()))?;
        fs::write(battery.join("status"), format!("{status}\n"))
            .with_context(|| format!("failed to write {}", battery.join("status").display()))?;
        Ok(())
    }
}

pub struct FakeProcIrq {
    root: TestRoot,
}

impl FakeProcIrq {
    pub fn new(name: &str) -> Self {
        Self {
            root: TestRoot::new(&format!("fake-proc-irq-{name}")),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn write_irq(
        &self,
        irq: u32,
        device_hint: &str,
        smp_affinity: &str,
    ) -> anyhow::Result<PathBuf> {
        let irq_dir = self.root.join(irq.to_string());
        fs::create_dir_all(&irq_dir)
            .with_context(|| format!("failed to create {}", irq_dir.display()))?;
        fs::write(irq_dir.join("actions"), format!("{device_hint}\n"))
            .with_context(|| format!("failed to write {}", irq_dir.join("actions").display()))?;
        fs::write(irq_dir.join("smp_affinity"), format!("{smp_affinity}\n")).with_context(
            || format!("failed to write {}", irq_dir.join("smp_affinity").display()),
        )?;
        Ok(irq_dir)
    }
}

pub struct FakeCgroupTree {
    root: TestRoot,
}

impl FakeCgroupTree {
    pub fn new(name: &str) -> Self {
        Self {
            root: TestRoot::new(&format!("fake-cgroup-{name}")),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn write_cgroup(&self, cgroup_path: &str) -> anyhow::Result<PathBuf> {
        let relative = cgroup_path.trim_start_matches('/');
        let path = if relative.is_empty() {
            self.root.path().to_path_buf()
        } else {
            self.root.join(relative)
        };

        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create fake cgroup {}", path.display()))?;
        fs::write(path.join("cgroup.procs"), "")
            .with_context(|| format!("failed to write {}", path.join("cgroup.procs").display()))?;
        fs::write(path.join("cpuset.cpus"), "0-7\n")
            .with_context(|| format!("failed to write {}", path.join("cpuset.cpus").display()))?;
        fs::write(path.join("cpuset.mems"), "0\n")
            .with_context(|| format!("failed to write {}", path.join("cpuset.mems").display()))?;

        Ok(path)
    }

    pub fn read_cgroup_procs(&self, cgroup_path: &str) -> anyhow::Result<String> {
        let relative = cgroup_path.trim_start_matches('/');
        let path = if relative.is_empty() {
            self.root.path().join("cgroup.procs")
        } else {
            self.root.join(relative).join("cgroup.procs")
        };

        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
    }
}

fn fake_stat_line(
    pid: u32,
    comm: &str,
    ppid: u32,
    starttime_ticks: u64,
    sched_policy: u32,
) -> String {
    let mut fields = vec!["0".to_owned(); 40];
    fields[0] = "S".to_owned();
    fields[1] = ppid.to_string();
    fields[19] = starttime_ticks.to_string();
    fields[38] = sched_policy.to_string();

    format!("{pid} ({comm}) {}", fields.join(" "))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        actions::{
            TaskIdentity,
            cgroup::{CgroupPlacementAction, CgroupPlacementPolicy, CgroupPlacementTarget},
            cpu_power::{BatteryPolicy, CpuPowerAction, CpuPowerPolicy},
            irq_affinity::{
                IrqAffinityAction, IrqAffinityEvidence, IrqAffinityPolicy, IrqAffinityRisk,
            },
        },
        process_tree::{TargetSnapshotInput, TaskClass, target_snapshot},
    };

    #[test]
    fn fake_proc_supports_task_identity_snapshot() {
        let proc = FakeProc::new("task-identity");
        proc.write_process(
            FakeProcess::new(100, "GameMain", 12_345)
                .ppid(1)
                .cgroup("/game.slice")
                .sched_policy(1)
                .thread(101, "RenderThread", 12_346),
        )
        .unwrap();

        let roots = [100];
        let snapshot = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc.path())
                .tree_pids(&roots),
        );

        assert!(snapshot.process_roots.contains(&100));
        assert!(snapshot.tasks.contains_key(&100));
        assert!(snapshot.tasks.contains_key(&101));

        let main = snapshot.tasks.get(&100).unwrap();
        assert_eq!(main.process_pid, 100);
        assert_eq!(main.process_ppid, 1);
        assert_eq!(main.comm, "GameMain");
        assert_eq!(main.process_starttime_ticks, Some(12_345));
        assert_eq!(main.task_starttime_ticks, Some(12_345));
        assert_eq!(main.sched_policy, Some(1));
        assert!(main.exe_dev.is_some());
        assert!(main.exe_ino.is_some());

        let render = snapshot.tasks.get(&101).unwrap();
        assert_eq!(render.process_pid, 100);
        assert_eq!(render.comm, "RenderThread");
        assert_eq!(render.process_starttime_ticks, Some(12_345));
        assert_eq!(render.task_starttime_ticks, Some(12_346));
        assert_eq!(render.sched_policy, Some(1));
    }

    #[test]
    fn fake_sysfs_supports_cpu_governor_and_epp_preflight() {
        let sysfs = FakeSysfs::new("governor-epp");
        sysfs
            .write_cpu_cpufreq(0, "schedutil", "balance_performance")
            .unwrap();
        sysfs.write_ac_power(true).unwrap();

        let action = CpuPowerAction {
            sysfs_root: sysfs.path().to_path_buf(),
            cpus: vec![0],
            scaling_governor: Some("performance".to_owned()),
            energy_performance_preference: Some("performance".to_owned()),
        };
        let policy = CpuPowerPolicy {
            allow_cpu_power_changes: true,
            allowed_cpus: [0].into_iter().collect::<BTreeSet<_>>(),
            allow_governor_changes: true,
            allow_epp_changes: true,
            battery_policy: BatteryPolicy::Never,
            explicit_battery_override: false,
        };

        let warnings = action.preflight_with_policy(&policy).unwrap();

        assert!(warnings.is_empty());
    }

    #[test]
    fn fake_sysfs_supports_battery_power_preflight() {
        let sysfs = FakeSysfs::new("battery-preflight");
        sysfs
            .write_cpu_cpufreq(0, "schedutil", "balance_performance")
            .unwrap();
        sysfs.write_battery("Discharging").unwrap();

        let action = CpuPowerAction {
            sysfs_root: sysfs.path().to_path_buf(),
            cpus: vec![0],
            scaling_governor: Some("performance".to_owned()),
            energy_performance_preference: Some("performance".to_owned()),
        };
        let policy = CpuPowerPolicy {
            allow_cpu_power_changes: true,
            allowed_cpus: [0].into_iter().collect::<BTreeSet<_>>(),
            allow_governor_changes: true,
            allow_epp_changes: true,
            battery_policy: BatteryPolicy::Never,
            explicit_battery_override: false,
        };

        let err = action.preflight_with_policy(&policy).unwrap_err();

        assert!(
            err.to_string().contains("while on battery"),
            "unexpected CPU power preflight error: {err:#}"
        );
    }

    #[test]
    fn fake_proc_irq_supports_irq_affinity_preflight() {
        let irq = FakeProcIrq::new("amdgpu");
        irq.write_irq(44, "amdgpu", "00000001").unwrap();

        let action = IrqAffinityAction {
            irq: 44,
            device_hint: "amdgpu".to_owned(),
            smp_affinity: "00000002".to_owned(),
            risk: IrqAffinityRisk::ReversibleMediumRisk,
            evidence: IrqAffinityEvidence {
                strong_irq_evidence: true,
                stable_irq_identity: true,
                known_device_mapping: true,
                observed_irq: Some(44),
                observed_device_hint: Some("amdgpu".to_owned()),
                reason: "fake test evidence ties IRQ 44 to amdgpu".to_owned(),
            },
            irq_root: irq.path().to_path_buf(),
        };
        let policy = IrqAffinityPolicy {
            allow_irq_affinity_changes: true,
            allow_high_risk_devices: false,
            require_strong_irq_evidence: true,
            require_stable_irq_identity: true,
            require_known_device_mapping: true,
        };

        let warnings = action.preflight_with_policy(&policy).unwrap();

        assert!(warnings.is_empty());
    }

    #[test]
    fn fake_cgroup_tree_supports_cgroup_placement_preflight() {
        let proc = FakeProc::new("cgroup-proc");
        let cgroup = FakeCgroupTree::new("placement");

        proc.write_task_identity(42, "game-thread", 12_345, "/old.slice")
            .unwrap();
        cgroup.write_cgroup("/old.slice").unwrap();
        cgroup.write_cgroup("/stutter/game.slice").unwrap();

        let action = CgroupPlacementAction {
            cgroup_root: cgroup.path().to_path_buf(),
            target_cgroup: PathBuf::from("/stutter/game.slice"),
            targets: vec![CgroupPlacementTarget {
                identity: TaskIdentity {
                    tid: 42,
                    process_pid: Some(42),
                    comm: Some("game-thread".to_owned()),
                    starttime_ticks: Some(12_345),
                },
                class: TaskClass::Game,
            }],
            cpuset_cpus: Some("2-3".to_owned()),
            cpuset_mems: Some("0".to_owned()),
        };

        let warnings = action
            .preflight_with_policy_at_for_tests(proc.path(), &CgroupPlacementPolicy::default())
            .unwrap();

        assert!(warnings.is_empty());
        assert_eq!(cgroup.read_cgroup_procs("/stutter/game.slice").unwrap(), "");
    }
}
