//! Independent telemetry for interventions and bucketed host facts. Everything here is pure
//! parsing/decision logic over text read from `/proc` and `/sys`, so it is tested without
//! hardware; the system host only feeds it real files.

/// Clock ticks per second used for `/proc/<pid>/stat` times (USER_HZ is 100 on Linux).
pub const CLOCK_TICKS: f64 = 100.0;

/// `(scheduling policy, rt priority, utime+stime ticks)` of a process from its stat line.
pub fn parse_sched(stat: &str) -> Option<(u32, u32, u64)> {
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    // Field N of proc(5) is index N-3 after the parenthesized command name.
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let rt_priority = fields.get(37)?.parse::<u32>().ok()?;
    let policy = fields.get(38)?.parse::<u32>().ok()?;
    Some((policy, rt_priority, utime + stime))
}

/// A scheduler burst is established if the contender ran as SCHED_FIFO at the requested
/// priority and consumed at least half the CPU it was meant to hold over the window.
pub fn scheduler_burst_established(
    stat: &str,
    priority: u32,
    burst_ms: u64,
    period_ms: u64,
    window_seconds: u64,
) -> Result<(), String> {
    let (policy, rt, ticks) =
        parse_sched(stat).ok_or("the contender's stat line is unreadable (it exited early)")?;
    if policy != 1 || rt != priority {
        return Err(format!(
            "contender policy {policy} priority {rt}, expected SCHED_FIFO {priority}"
        ));
    }
    let expected = window_seconds as f64 * CLOCK_TICKS * burst_ms as f64 / period_ms.max(1) as f64;
    if (ticks as f64) < expected * 0.5 {
        return Err(format!(
            "contender used {ticks} ticks of CPU, below half of the expected {expected:.0}"
        ));
    }
    Ok(())
}

/// GPU saturation is established if enough samples were taken and their mean reached `min_pct`.
pub fn gpu_busy_established(
    samples: &[u32],
    min_pct: u32,
    min_samples: usize,
) -> Result<(), String> {
    if samples.len() < min_samples {
        return Err(format!(
            "{} GPU busy samples, fewer than the required {min_samples}",
            samples.len()
        ));
    }
    let mean = samples.iter().map(|s| u64::from(*s)).sum::<u64>() / samples.len() as u64;
    if mean < u64::from(min_pct) {
        return Err(format!(
            "mean GPU busy {mean}% is below the required {min_pct}%"
        ));
    }
    Ok(())
}

/// First element of `ladder` for which `probe` succeeds (a bounded calibration walk). `Err`
/// stops the walk (a probe could not run at all); `Ok(None)` means nothing in the ladder works.
pub fn first_passing<T>(
    ladder: &[T],
    probe: &mut dyn FnMut(&T) -> Result<bool, String>,
) -> Result<Option<usize>, String> {
    for (index, step) in ladder.iter().enumerate() {
        if probe(step)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

pub fn gpu_vendor(pci_vendor_id: &str) -> &'static str {
    match pci_vendor_id.trim().to_ascii_lowercase().as_str() {
        "0x1002" => "AMD",
        "0x10de" => "NVIDIA",
        "0x8086" => "Intel",
        _ => "Unknown",
    }
}

/// `ID` and `VERSION_ID` only, as the other evidence does.
pub fn os_release_summary(text: &str) -> String {
    text.lines()
        .filter(|line| line.starts_with("ID=") || line.starts_with("VERSION_ID="))
        .map(|line| line.replace('"', ""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Vendor and a `<cores>c<threads>t` bucket; the model string never leaves the machine.
pub fn cpu_bucket(cpuinfo: &str) -> (String, String) {
    let vendor = cpuinfo
        .lines()
        .find_map(|l| {
            l.strip_prefix("vendor_id")
                .and_then(|r| r.split_once(':'))
                .map(|(_, v)| v.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let threads = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    let cores = cpuinfo
        .lines()
        .find_map(|l| {
            l.strip_prefix("cpu cores")
                .and_then(|r| r.split_once(':'))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(threads);
    (vendor, format!("{cores}c{threads}t"))
}
