//! Privileged worker startup checks for autotune runtime sessions.

use std::time::Duration;

use crate::{
    autotune::runtime::AutotuneRuntimeConfig,
    daemon::{
        policy::DaemonMode,
        privilege::{PrivilegedWorkerHandle, default_privileged_worker_socket_path},
    },
};

pub(crate) fn maybe_spawn_privileged_worker(
    config: &AutotuneRuntimeConfig,
) -> anyhow::Result<Option<PrivilegedWorkerHandle>> {
    if config.mode() != DaemonMode::ApplyMediumRisk
        || config.simulate_action_effects
        || config
            .daemon_config
            .autotune
            .unsafe_in_process_privileged_worker
        || !config.daemon_config.autotune.manage_privileged_worker
    {
        return Ok(None);
    }

    let socket_path = config
        .daemon_config
        .autotune
        .privileged_worker_socket
        .clone()
        .map(Ok)
        .unwrap_or_else(default_privileged_worker_socket_path)?;
    let socket_ready_timeout = Duration::from_millis(
        config
            .daemon_config
            .autotune
            .privileged_worker_socket_ready_timeout_ms,
    );
    let socket_ready_retry_interval = Duration::from_millis(
        config
            .daemon_config
            .autotune
            .privileged_worker_socket_ready_retry_ms,
    );
    PrivilegedWorkerHandle::spawn_with_timing(
        &socket_path,
        socket_ready_timeout,
        socket_ready_retry_interval,
    )
    .map(Some)
}

pub(crate) fn warn_if_unmanaged_privileged_worker_missing(
    config: &AutotuneRuntimeConfig,
) -> anyhow::Result<()> {
    if config.mode() != DaemonMode::ApplyMediumRisk
        || config.simulate_action_effects
        || config
            .daemon_config
            .autotune
            .unsafe_in_process_privileged_worker
        || config.daemon_config.autotune.manage_privileged_worker
    {
        return Ok(());
    }

    let socket_path = config
        .daemon_config
        .autotune
        .privileged_worker_socket
        .clone()
        .map(Ok)
        .unwrap_or_else(default_privileged_worker_socket_path)?;
    if !socket_path.exists() {
        let message = format!(
            "ApplyMediumRisk is configured with managed privileged worker disabled, but socket {} is missing; start it with: stutter privileged-worker --socket {}",
            socket_path.display(),
            socket_path.display()
        );
        log::warn!("{message}");
    }
    Ok(())
}
