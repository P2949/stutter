//! Facts frozen once per study so every capture and every resumed run uses the same ones:
//! the permitted study CPU, the NIC interrupt with the CPU that actually services it, and the
//! number of I/Os that makes the fixed-work target last long enough to be measured.
use std::{fs, path::Path, time::Duration};

use super::store::Frozen;
use crate::validation_campaign::{
    host_factory::{flow_plan, msi_irq_names, probe_irq, read_interrupts},
    workloads::{
        CommandSpec, SystemTools, ToolHost,
        fio::{self, FioProfile},
        network_irq,
    },
};

/// Target runtime of the unloaded fixed-work reader.
const FIXED_WORK_SECONDS: f64 = 20.0;
const FIXED_WORK_MIN_IOS: u64 = 20_000;
const FIXED_WORK_MAX_IOS: u64 = 3_000_000;
const PROBE_SECONDS: u64 = 6;

/// `0-3,6` -> `[0, 1, 2, 3, 6]`; `None` for anything that is not a kernel CPU list.
pub fn parse_cpu_list(list: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in list.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((lo, hi)) => {
                let (lo, hi): (u32, u32) = (lo.parse().ok()?, hi.parse().ok()?);
                if lo > hi {
                    return None;
                }
                cpus.extend(lo..=hi);
            }
            None => cpus.push(part.parse().ok()?),
        }
    }
    (!cpus.is_empty()).then_some(cpus)
}

/// The first permitted CPU that is neither CPU 0 (where housekeeping gathers) nor the one that
/// services the NIC interrupt, so the scheduler study and the IRQ study never share a CPU.
pub fn choose_study_cpu(allowed: &[u32], irq_cpu: u32) -> Option<u32> {
    allowed.iter().copied().find(|c| *c != 0 && *c != irq_cpu)
}

/// CPUs for the traffic senders: permitted, not CPU 0, and sharing no core with the target's CPU
/// or the CPU that services the interrupt (`siblings` lists each excluded CPU's SMT siblings).
pub fn choose_traffic_cpus(allowed: &[u32], excluded: &[u32], want: usize) -> Vec<u32> {
    allowed
        .iter()
        .copied()
        .filter(|c| *c != 0 && !excluded.contains(c))
        .take(want)
        .collect()
}

fn siblings_of(cpu: u32) -> Vec<u32> {
    fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"
    ))
    .ok()
    .and_then(|list| parse_cpu_list(&list))
    .unwrap_or_else(|| vec![cpu])
}

/// Fixed-work size from an unloaded I/O rate, so the control run lasts about 20 s.
pub fn fixed_work_ios(iops: f64) -> u64 {
    ((iops * FIXED_WORK_SECONDS) as u64).clamp(FIXED_WORK_MIN_IOS, FIXED_WORK_MAX_IOS)
}

fn allowed_cpus() -> Option<Vec<u32>> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .and_then(parse_cpu_list)
}

fn per_cpu_delta(
    before: &std::collections::BTreeMap<u32, Vec<u64>>,
    after: &std::collections::BTreeMap<u32, Vec<u64>>,
    irq: u32,
) -> Option<Vec<u64>> {
    let (b, a) = (before.get(&irq)?, after.get(&irq)?);
    Some(a.iter().zip(b).map(|(a, b)| a.saturating_sub(*b)).collect())
}

/// Chooses the NIC interrupt (as the v0.1 corpus did) and proves which CPU services it: the
/// kernel's effective affinity must be a single CPU and the traffic-driven per-CPU increase
/// must land there.
fn freeze_irq(frozen: &mut Frozen) -> Result<(), String> {
    let nic = network_irq::discover(&SystemTools, &|iface| msi_irq_names(iface))?;
    let irq = probe_irq(&nic)?;
    let affinity = fs::read_to_string(format!("/proc/irq/{irq}/effective_affinity_list"))
        .map_err(|e| format!("IRQ {irq} has no effective affinity: {e}"))?;
    let cpus = parse_cpu_list(&affinity).ok_or("unreadable effective affinity")?;
    let [irq_cpu] = cpus.as_slice() else {
        return Err(format!(
            "IRQ {irq} is serviced on {cpus:?}, not on a single CPU: the IRQ study needs one"
        ));
    };
    let target = std::net::SocketAddr::from((nic.gateway, 9));
    let socket = network_irq::steer_flow(target, irq, 40, flow_plan(), &read_interrupts)?;
    let read = || {
        fs::read_to_string("/proc/interrupts")
            .map(|t| network_irq::parse_interrupts_per_cpu(&t))
            .map_err(|e| e.to_string())
    };
    let before = read()?;
    network_irq::pump(&socket, 4_000, 512, Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(200));
    let deltas = per_cpu_delta(&before, &read()?, irq).ok_or("the IRQ vanished")?;
    let busiest = deltas
        .iter()
        .enumerate()
        .max_by_key(|(_, d)| **d)
        .map(|(cpu, _)| cpu as u32);
    if busiest != Some(*irq_cpu) {
        return Err(format!(
            "IRQ {irq}: effective affinity says CPU {irq_cpu} but traffic landed on {busiest:?} ({deltas:?})"
        ));
    }
    frozen.irq = Some(irq);
    frozen.irq_cpu = Some(*irq_cpu);
    Ok(())
}

fn freeze_fixed_work(scratch: &Path) -> Result<u64, String> {
    fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let profile = FioProfile::target_direct();
    let command = CommandSpec {
        program: "fio".to_owned(),
        args: vec![
            "--name=probe".to_owned(),
            format!("--directory={}", scratch.display()),
            "--output-format=json".to_owned(),
            "--rw=randread".to_owned(),
            format!("--bs={}", profile.block_size),
            format!("--size={}", profile.file_size),
            "--ioengine=psync".to_owned(),
            "--direct=1".to_owned(),
            "--time_based".to_owned(),
            format!("--runtime={PROBE_SECONDS}"),
        ],
        env: Vec::new(),
        cwd: None,
    };
    let output = SystemTools.output(&command)?;
    let summary = fio::summarize(&output)?;
    if summary.errors > 0 || summary.total_ios == 0 {
        return Err("the unloaded fixed-work probe did no I/O".to_owned());
    }
    Ok(fixed_work_ios(
        summary.total_ios as f64 / PROBE_SECONDS as f64,
    ))
}

/// Freezes whatever is not frozen yet; `Err` is an honest resource block.
pub fn freeze_missing(frozen: &mut Frozen, scratch: &Path) -> Result<(), String> {
    if frozen.irq.is_none() || frozen.irq_cpu.is_none() {
        freeze_irq(frozen)?;
    }
    if frozen.study_cpu.is_none() {
        let irq_cpu = frozen.irq_cpu.ok_or("no IRQ CPU")?;
        let allowed = allowed_cpus().ok_or("the permitted CPU list is unreadable")?;
        frozen.study_cpu = Some(
            choose_study_cpu(&allowed, irq_cpu)
                .ok_or_else(|| format!("no permitted CPU other than 0 and {irq_cpu}"))?,
        );
    }
    if frozen.traffic_cpus.is_empty() {
        let allowed = allowed_cpus().ok_or("the permitted CPU list is unreadable")?;
        let mut excluded = Vec::new();
        for cpu in [frozen.study_cpu, frozen.irq_cpu].into_iter().flatten() {
            excluded.extend(siblings_of(cpu));
        }
        frozen.traffic_cpus = choose_traffic_cpus(&allowed, &excluded, 3);
        if frozen.traffic_cpus.is_empty() {
            return Err("no permitted CPU is free of the target and the interrupt".to_owned());
        }
    }
    if frozen.fixed_work_ios.is_none() {
        frozen.fixed_work_ios = Some(freeze_fixed_work(scratch)?);
    }
    Ok(())
}
