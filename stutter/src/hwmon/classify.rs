use std::{
    fs,
    sync::{Arc, OnceLock, atomic::Ordering},
    time::Duration,
};

use super::model::{NvidiaSample, NvidiaState, NvidiaWorker};

pub(super) fn has_nvidia_pci_device() -> bool {
    static NVIDIA_PCI_PRESENT: OnceLock<bool> = OnceLock::new();

    *NVIDIA_PCI_PRESENT.get_or_init(has_nvidia_pci_device_uncached)
}

fn has_nvidia_pci_device_uncached() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };
    for entry in entries.flatten() {
        if let Ok(vendor) = fs::read_to_string(entry.path().join("vendor"))
            && vendor.trim() == "0x10de"
        {
            return true;
        }
    }
    false
}

pub(super) fn start_nvidia_smi_thread() -> NvidiaWorker {
    let state = Arc::new(NvidiaState::new());

    let state_clone = state.clone();
    std::thread::spawn(move || {
        while !state_clone.shutdown.load(Ordering::Relaxed) {
            let output = std::process::Command::new("nvidia-smi")
                .args([
                    "--query-gpu=utilization.gpu,memory.used,memory.total",
                    "--format=csv,noheader,nounits",
                ])
                .output();

            if let Ok(out) = output
                && let Ok(s) = String::from_utf8(out.stdout)
                && let Some(sample) = parse_nvidia_smi_sample(&s)
                && let Ok(mut latest) = state_clone.latest.lock()
            {
                *latest = Some(sample);
            }

            // Sleep in small increments but with a larger total interval to
            // avoid spawning `nvidia-smi` frequently. Default total wait is
            // 5s, checked every 100ms so the worker can shut down promptly.
            let total_ms = 5_000u64;
            let step_ms = 100u64;
            let iterations = (total_ms / step_ms) as usize;
            for _ in 0..iterations {
                if state_clone.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(step_ms));
            }
        }
    });

    NvidiaWorker { state }
}

pub(super) fn parse_nvidia_smi_sample(output: &str) -> Option<NvidiaSample> {
    let line = output.lines().next()?;
    let mut parts = line.split(',').map(str::trim);
    let busy = parts.next()?.parse::<u32>().ok()?;
    let used_mb = parts.next()?.parse::<u64>().ok()?;
    let total_mb = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    Some(NvidiaSample {
        gpu_busy_percent: busy,
        vram_used_bytes: used_mb * 1024 * 1024,
        vram_total_bytes: total_mb * 1024 * 1024,
    })
}
