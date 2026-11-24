//! The crosswalk's observations, derived from the campaign's own sealed raw evidence rather than asserted: at
//! finalization every child's `resident-windows.json` (the resident runtime's raw per-attempt counters) is read back,
//! and a dossier whose own signal the runtime really received for the whole of every attempt of every measuring child is
//! written down as observed, with an immutable, content-named evidence artifact (`crosswalk-evidence-<sha>.json`) the
//! report later reopens and verifies. Only dossiers whose signal the runtime itself feeds are produced here:
//!
//! * 5.22 PSI: pressure readings in the game's interval records (`psi_samples`).
//! * 5.13 GPU owner/observe: GPU samples (`gpu_samples`).
//! * 5.17 CPU frequency and power guardrail: CPU frequency events (`cpu_freq_samples`).
//! * 5.9 CPU topology, 5.19 one online NUMA node and 5.20 memory guardrail: the kernel's own files (`resident_observer::host_probe`),
//!   looked at once per interval in which the runtime saw the game.
//!
//! A dossier with a single attempt short of the sample floor, or no window at all, is not produced: absence stays
//! `NotObservedInThisCampaign`. Nothing here upgrades a dossier from a plan or a fixture.

use std::{fs::File, os::fd::AsFd};

use serde_json::{Value, json};

use super::{
    crosswalk::OBSERVATIONS_FILE,
    crosswalk_evidence::file_name,
    report_store::{read_private, write_new},
    resident_observation::MIN_SAMPLES,
    resident_observer::WINDOWS_FILE,
};

/// The dossiers produced, with the provider that fed each and the raw counter that shows it.
const PRODUCED: [(&str, &str, &str); 13] = [
    ("5.22", "psi-interval-records", "psi_samples"),
    ("5.13", "gpu-sample-events", "gpu_samples"),
    ("5.17", "cpu-frequency-events", "cpu_freq_samples"),
    ("5.9", "host-cpu-topology-files", "topology_samples"),
    ("5.19", "host-numa-node-online", "single_numa_samples"),
    ("5.20", "host-memory-guard-files", "memory_guard_samples"),
    ("5.18", "host-irq-affinity-files", "irq_placement_samples"),
    ("5.16", "host-power-policy-files", "power_policy_samples"),
    ("5.6", "host-sched-ext-state-file", "sched_ext_samples"),
    ("5.8", "host-damon-admin-interface", "damon_samples"),
    ("5.24", "host-cpu-rdt-flags", "resctrl_absent_samples"),
    ("5.26", "host-io-pressure-files", "io_pressure_samples"),
    ("extra.boot", "host-kernel-cmdline", "boot_cmdline_samples"),
];

/// Whether the resident runtime produces observation evidence for this dossier at all.
pub(super) fn produces(id: &str) -> bool {
    PRODUCED.iter().any(|produced| produced.0 == id)
}

/// Write the campaign's observations record and its evidence artifacts from the children's raw windows.
pub fn produce(campaign: &File, uid: u32, campaign_sha256: &str) -> anyhow::Result<()> {
    let Ok(studies) =
        crate::syscall::openat_dir_nofollow(campaign.as_fd(), "studies").map(File::from)
    else {
        return Ok(());
    };
    let names: Vec<String> = std::fs::read_dir(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&studies)
    ))?
    .flatten()
    .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
    .collect();
    let mut windows: Vec<Value> = Vec::new();
    // The raw window files the observations stand on, by child and digest: the evidence names each, so it can be
    // reopened and checked against exactly what was read.
    let mut sources: Vec<Value> = Vec::new();
    for name in names {
        let Ok(child) = crate::syscall::openat_dir_nofollow(studies.as_fd(), &name).map(File::from)
        else {
            continue;
        };
        // A child with no window file kept none (it never observed a game). One whose file is there but cannot be read
        // whole is a gap in the coverage, never a silent skip.
        let bytes = match read_private(&child, WINDOWS_FILE, uid) {
            Ok(bytes) => bytes,
            Err(error) if is_absent(&error) => continue,
            Err(_) => anyhow::bail!("CrosswalkChildWindowsUnreadable"),
        };
        let Ok(Value::Array(rows)) = serde_json::from_slice::<Value>(&bytes) else {
            anyhow::bail!("CrosswalkChildWindowsUnreadable");
        };
        sources.push(json!({ "child": name, "sha256": stutter_core::hash::sha256_hex(&bytes) }));
        windows.extend(rows);
    }
    if windows.is_empty() {
        return Ok(());
    }
    let (start, end) = (
        windows
            .iter()
            .filter_map(|w| w["started_unix"].as_u64())
            .min(),
        windows
            .iter()
            .filter_map(|w| w["ended_unix"].as_u64())
            .max(),
    );
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(());
    };
    if start >= end {
        return Ok(());
    }
    let mut rows = Vec::new();
    for (id, provider, counter) in PRODUCED {
        let every_attempt = windows
            .iter()
            .all(|w| w[counter].as_u64().unwrap_or(0) >= MIN_SAMPLES);
        if !every_attempt {
            continue;
        }
        let artifact = serde_json::to_vec(&json!({
            "schema": 1, "dossier": id, "campaign_sha256": campaign_sha256, "provider": provider,
            "source_quality": "direct", "start_unix": start, "end_unix": end, "sources": sources,
        }))?;
        let digest = stutter_core::hash::sha256_hex(&artifact);
        write_once(campaign, &file_name(&digest), &artifact)?;
        rows.push(json!({ "id": id, "evidence_sha256": digest }));
    }
    if rows.is_empty() {
        return Ok(());
    }
    write_once(
        campaign,
        OBSERVATIONS_FILE,
        &serde_json::to_vec(&json!({ "rows": rows }))?,
    )
}

fn write_once(dir: &File, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    match write_new(dir, name, bytes) {
        Ok(()) => Ok(()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Whether a read failed only because the file is not there.
fn is_absent(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
#[path = "crosswalk_producer_tests.rs"]
mod tests;
