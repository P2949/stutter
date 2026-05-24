//! Gated real eBPF loader tests.
//!
//! These tests are visible in normal test runs and skip unless explicitly
//! enabled with `STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1`.

use std::path::Path;

use crate::{config::model::MonitorConfig, session::targeting::TargetPolicy};

fn privileged_ebpf_tests_enabled() -> bool {
    std::env::var_os("STUTTER_RUN_PRIVILEGED_EBPF_TESTS").is_some()
}

fn skip_unless_privileged_ebpf_enabled() -> bool {
    if !privileged_ebpf_tests_enabled() {
        eprintln!("skipped: set STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1");
        return true;
    }
    false
}

fn require_linux() -> anyhow::Result<()> {
    anyhow::ensure!(cfg!(target_os = "linux"), "requires Linux");
    Ok(())
}

fn require_tracefs() -> anyhow::Result<()> {
    let candidates = [
        "/sys/kernel/tracing/events",
        "/sys/kernel/debug/tracing/events",
    ];

    if candidates.iter().any(|path| Path::new(path).exists()) {
        return Ok(());
    }

    anyhow::bail!("tracefs events directory not found");
}

fn require_bpf_permissions() -> anyhow::Result<()> {
    anyhow::ensure!(
        privileged_ebpf_tests_enabled(),
        "set STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1"
    );
    Ok(())
}

fn load_attach_and_drop(config: &MonitorConfig) -> anyhow::Result<()> {
    require_linux()?;
    require_tracefs()?;
    require_bpf_permissions()?;

    let target_policy = TargetPolicy::from_monitor_config(config)?;
    let loaded = crate::ebpf::load::load_and_attach(config, &target_policy)?;
    drop(loaded);

    Ok(())
}

#[tokio::test]
async fn privileged_load_attach_and_drop_default_config() -> anyhow::Result<()> {
    if skip_unless_privileged_ebpf_enabled() {
        return Ok(());
    }

    load_attach_and_drop(&MonitorConfig::default())
}

#[tokio::test]
async fn privileged_load_drop_load_again_default_config() -> anyhow::Result<()> {
    if skip_unless_privileged_ebpf_enabled() {
        return Ok(());
    }

    for _ in 0..2 {
        load_attach_and_drop(&MonitorConfig::default())?;
    }

    Ok(())
}

#[tokio::test]
async fn privileged_load_accepts_custom_map_sizing() -> anyhow::Result<()> {
    if skip_unless_privileged_ebpf_enabled() {
        return Ok(());
    }

    let mut config = MonitorConfig::default();
    config.ebpf_sizing.ringbuf_size_kb = Some(256);
    config.ebpf_sizing.wakeup_map_factor = Some(256);
    config.ebpf_sizing.target_irqs_entries = Some(128);
    config.ebpf_sizing.irq_start_entries = Some(4_096);
    config.ebpf_sizing.block_start_entries = Some(32_768);
    config.ebpf_sizing.kms_flip_start_entries = Some(8_192);
    config.ebpf_sizing.drm_fence_wait_start_entries = Some(8_192);
    config.ebpf_sizing.drm_fence_signal_entries = Some(8_192);

    load_attach_and_drop(&config)
}

#[tokio::test]
async fn privileged_load_with_optional_latency_probes_degrades_cleanly() -> anyhow::Result<()> {
    if skip_unless_privileged_ebpf_enabled() {
        return Ok(());
    }

    let mut config = MonitorConfig::default();
    config.probes.block_io = true;
    config.probes.kms_timing = true;
    config.probes.drm_fence_latency = true;
    config.probes.irq_latency = true;

    load_attach_and_drop(&config)
}
