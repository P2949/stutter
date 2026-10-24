//! Study-only host behaviour: explicit CPU placement of owned targets (never privileged
//! mutation), the fixed-work target's completion and direct effect metric, and per-CPU
//! interrupt verification. Every check reads independent telemetry (`/proc`, fio JSON).
use std::collections::BTreeMap;

use super::*;

/// How long a fixed-work target may keep running after the recording ends.
pub(super) const FIXED_WORK_WAIT_SECONDS: u64 = 300;

pub(super) fn read_per_cpu() -> Result<BTreeMap<u32, Vec<u64>>, String> {
    fs::read_to_string("/proc/interrupts")
        .map(|text| network_irq::parse_interrupts_per_cpu(&text))
        .map_err(|e| e.to_string())
}

/// Longest a capture waits for the host to settle after the previous one.
const QUIESCE_CAP_SECONDS: u64 = 240;
/// Pressure (percent of the last 10 s) and unwritten data below which storage counts as idle.
const QUIET_IO_PSI_PERCENT: f64 = 1.0;
const QUIET_DIRTY_KB: u64 = 64 * 1024;
const QUIET_GPU_BUSY_PERCENT: u32 = 10;
const QUIET_SAMPLES: u32 = 3;

/// `some avg10` of a `/proc/pressure/*` file.
pub fn psi_some_avg10(text: &str) -> Option<f64> {
    text.lines()
        .find(|line| line.starts_with("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// A `/proc/meminfo` value in kB.
pub fn meminfo_kb(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Is the host idle enough that the previous capture cannot leak into the next one?
pub fn host_is_quiet(io_psi: Option<f64>, dirty_kb: Option<u64>, gpu: Option<u32>) -> bool {
    io_psi.is_some_and(|p| p < QUIET_IO_PSI_PERCENT)
        && dirty_kb.is_some_and(|k| k < QUIET_DIRTY_KB)
        && gpu.is_none_or(|g| g < QUIET_GPU_BUSY_PERCENT)
}

/// `Cpus_allowed_list` of a process, as written by the kernel.
fn allowed_cpus(pid: u32) -> Option<String> {
    read_proc(pid, "status")?
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .map(|list| list.trim().to_owned())
}

fn effective_irq_cpus(irq: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/irq/{irq}/effective_affinity_list"))
        .ok()
        .map(|list| list.trim().to_owned())
}

impl SystemCaptureHost {
    /// Blocks until storage and the GPU have settled after the previous capture: a blocker's
    /// writeback or a stressor's clocks would otherwise leak into the next run and bias a
    /// matched pair. `Err` when the host never settles; the capture is then rejected, not run.
    pub(super) fn wait_until_quiet(&self) -> Result<(), String> {
        let mut consecutive = 0;
        let mut last = String::new();
        for _ in 0..QUIESCE_CAP_SECONDS {
            let io = fs::read_to_string("/proc/pressure/io").ok();
            let mem = fs::read_to_string("/proc/meminfo").ok().unwrap_or_default();
            let dirty = meminfo_kb(&mem, "Dirty")
                .zip(meminfo_kb(&mem, "Writeback"))
                .map(|(d, w)| d + w);
            let io_psi = io.as_deref().and_then(psi_some_avg10);
            let gpu = gpu_busy_percent();
            last = format!("io pressure {io_psi:?}, unwritten {dirty:?} kB, gpu busy {gpu:?}");
            consecutive = if host_is_quiet(io_psi, dirty, gpu) {
                consecutive + 1
            } else {
                0
            };
            if consecutive >= QUIET_SAMPLES {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(format!(
            "the host did not settle within {QUIESCE_CAP_SECONDS} s ({last})"
        ))
    }

    /// The CPU the scenario puts its target on, if any.
    pub(super) fn placement_cpu(&self, spec: &ScenarioSpec) -> Result<Option<u32>, String> {
        match spec.placement {
            Placement::Unpinned => Ok(None),
            Placement::StudyCpu => self
                .settings
                .study_cpu
                .map(Some)
                .ok_or_else(|| "no study CPU was frozen".to_owned()),
            Placement::IrqCpu => self
                .settings
                .irq_cpu
                .map(Some)
                .ok_or_else(|| "no IRQ CPU was frozen".to_owned()),
        }
    }

    /// Launches the owned child under an explicit CPU mask so descendants inherit it.
    pub(super) fn pinned(
        &self,
        spec: &ScenarioSpec,
        command: CommandSpec,
    ) -> Result<CommandSpec, String> {
        let Some(cpu) = self.placement_cpu(spec)? else {
            return Ok(command);
        };
        let mut args = vec!["-c".to_owned(), cpu.to_string(), command.program];
        args.extend(command.args);
        Ok(CommandSpec {
            program: "taskset".to_owned(),
            args,
            env: command.env,
            cwd: command.cwd,
        })
    }

    /// Where a scheduler contender goes: the target's pinned CPU, else wherever it last ran.
    pub(super) fn burst_cpu(
        &self,
        spec: &ScenarioSpec,
        target: TargetIdentity,
    ) -> Result<u32, String> {
        if let Some(cpu) = self.placement_cpu(spec)? {
            return Ok(cpu);
        }
        read_proc(target.pid, "stat")
            .and_then(|stat| last_cpu(&stat))
            .ok_or_else(|| "cannot read the target's CPU".to_owned())
    }

    /// The target is still confined to its CPU (and, for an IRQ placement, the interrupt is
    /// still serviced there). Run while the target is alive.
    pub(super) fn check_placement(&mut self, spec: &ScenarioSpec) -> Result<(), String> {
        let Some(cpu) = self.placement_cpu(spec)? else {
            return Ok(());
        };
        let pid = self.target_pid.ok_or("the target was never started")?;
        let allowed = allowed_cpus(pid).ok_or("the target's CPU mask is unreadable")?;
        let mut observed = serde_json::json!({"cpu": cpu, "cpus_allowed": allowed});
        let mut result = if allowed == cpu.to_string() {
            Ok(())
        } else {
            Err(format!(
                "the target's CPU mask is {allowed}, expected {cpu}"
            ))
        };
        if spec.placement == Placement::IrqCpu {
            let irq = self.settings.irq.ok_or("no IRQ was frozen")?;
            let effective = effective_irq_cpus(irq);
            observed["irq_effective_affinity"] = serde_json::json!(effective);
            if effective.as_deref() != Some(cpu.to_string().as_str()) {
                result = Err(format!(
                    "IRQ {irq} is serviced on {effective:?}, not on CPU {cpu}: the relation moved"
                ));
            }
        }
        self.observed.insert("placement".to_owned(), observed);
        self.placement_result = Some(result.clone());
        result
    }

    /// Waits for a fixed-work target to finish and records how long its work took.
    pub(super) fn wait_fixed_work(&mut self, spec: &ScenarioSpec) -> Result<(), String> {
        if spec.target != Target::FioFixedWorkV2 {
            return Ok(());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(FIXED_WORK_WAIT_SECONDS);
        loop {
            let finished = self
                .children
                .first_mut()
                .ok_or("the fixed-work target is gone")?
                .try_wait()
                .map_err(|e| e.to_string())?;
            if finished.is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "the fixed-work target did not finish within {FIXED_WORK_WAIT_SECONDS} s"
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let text = fs::read_to_string(self.capture_dir.join("target.log"))
            .map_err(|e| format!("no target output: {e}"))?;
        let summary = fio::fixed_work_summary(&text)?;
        self.observed.insert(
            "target_elapsed_ms".to_owned(),
            serde_json::json!(summary.elapsed_ms),
        );
        self.observed.insert(
            "target_ios".to_owned(),
            serde_json::json!(summary.total_ios),
        );
        self.observed.insert(
            "target_p99_clat_ns".to_owned(),
            serde_json::json!(summary.p99_clat_ns),
        );
        Ok(())
    }

    pub(super) fn note_gpu_busy(&mut self) {
        if !self.gpu_samples.is_empty() {
            let mean =
                f64::from(self.gpu_samples.iter().sum::<u32>()) / self.gpu_samples.len() as f64;
            self.observed
                .insert("gpu_busy_mean".to_owned(), serde_json::json!(mean));
        }
    }

    /// The per-CPU interrupt count of the frozen IRQ rose on its servicing CPU, and that CPU
    /// carried more of the increase than any other.
    pub(super) fn verify_irq_on_cpu(&mut self) -> Result<(), String> {
        let irq = self.settings.irq.ok_or("no IRQ was frozen")?;
        let cpu = self.settings.irq_cpu.ok_or("no IRQ CPU was frozen")? as usize;
        let before = self
            .irq_cpu_before
            .as_ref()
            .and_then(|all| all.get(&irq))
            .ok_or("no per-CPU interrupt baseline")?;
        let after_all = read_per_cpu()?;
        let after = after_all
            .get(&irq)
            .ok_or("the IRQ is not in /proc/interrupts")?;
        let deltas: Vec<u64> = after
            .iter()
            .zip(before)
            .map(|(a, b)| a.saturating_sub(*b))
            .collect();
        self.observed.insert(
            "irq".to_owned(),
            serde_json::json!({"irq": irq, "cpu": cpu, "per_cpu_delta": deltas}),
        );
        let on_cpu = deltas.get(cpu).copied().unwrap_or(0);
        let busiest = deltas.iter().copied().max().unwrap_or(0);
        if on_cpu < MIN_IRQ_DELTA {
            return Err(format!(
                "IRQ {irq} rose by {on_cpu} on CPU {cpu} (< {MIN_IRQ_DELTA}); per-CPU {deltas:?}"
            ));
        }
        if on_cpu < busiest {
            return Err(format!(
                "IRQ {irq} moved: CPU {cpu} saw {on_cpu} but another CPU saw {busiest}"
            ));
        }
        Ok(())
    }
}
