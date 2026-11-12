//! Autotune controller session loop and shutdown result handling.

use std::{sync::Arc, time::Duration};

use tokio::sync::{mpsc, oneshot};

use crate::{
    autotune::runtime::{
        AutotuneDecisionStreamEntry, AutotuneRuntime, AutotuneRuntimeConfig,
        validate_runtime_config,
        worker::{maybe_spawn_privileged_worker, warn_if_unmanaged_privileged_worker_missing},
    },
    config::model::MonitorConfig,
    session_events::MonitorEvent,
};

#[derive(Debug)]
pub struct AutotuneControllerExit {
    pub reason: String,
    pub last_decision: Option<AutotuneDecisionStreamEntry>,
}

pub async fn run_autotune_controller_session(
    monitor_config: Arc<MonitorConfig>,
    runtime_config: AutotuneRuntimeConfig,
    external_stop: Option<oneshot::Receiver<()>>,
    duration: Option<Duration>,
) -> anyhow::Result<AutotuneControllerExit> {
    validate_runtime_config(&runtime_config).map_err(anyhow::Error::new)?;
    let (event_tx, mut event_rx) = mpsc::channel::<MonitorEvent>(1024);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let mut runtime = AutotuneRuntime::new(runtime_config);
    let (research_tx, mut research_rx) = mpsc::channel(8);
    let _research_server = match &runtime.config().daemon_config.autotune.research_socket_dir {
        Some(dir) => Some(super::research_control::ResearchServer::bind(dir, research_tx).await?),
        None => {
            drop(research_tx);
            None
        }
    };
    let mut research_coordinator = super::research_control::ResearchCoordinator::new(
        runtime
            .config()
            .daemon_config
            .autotune
            .research_campaign_dir
            .clone(),
        None,
        "BlockedHostIdentity",
    );
    let (host_context_tx, mut host_context_rx) = mpsc::channel(1);
    let (recovery_tx, mut recovery_rx) = mpsc::channel(1);
    let mut host_context_refresh = if runtime
        .config()
        .daemon_config
        .autotune
        .research_socket_dir
        .is_some()
    {
        let context = runtime.config().clone();
        let validation_override_present = context.validation_override.is_some();
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let snapshot = context.clone();
                let result = match tokio::task::spawn_blocking(move || {
                    super::super::research::host::collect_campaign_facts(
                        &snapshot,
                        validation_override_present,
                        false,
                    )
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => continue,
                };
                if host_context_tx.send(result).await.is_err() {
                    break;
                }
            }
        }))
    } else {
        drop(host_context_tx);
        None
    };
    warn_if_unmanaged_privileged_worker_missing(runtime.config())?;
    let mut worker_handle = maybe_spawn_privileged_worker(runtime.config())?;
    let mut event_loop_iterations = 0u64;
    let mut research_deadline_tick = tokio::time::interval(Duration::from_secs(1));
    research_deadline_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let (stop_task, _stop_tx_guard) = if duration.is_some() || external_stop.is_some() {
        (
            Some(tokio::spawn(async move {
                match (duration, external_stop) {
                    (Some(duration), Some(mut external_stop)) => {
                        tokio::select! {
                            _ = tokio::time::sleep(duration) => {}
                            _ = &mut external_stop => {}
                        }
                    }
                    (Some(duration), None) => {
                        tokio::time::sleep(duration).await;
                    }
                    (None, Some(mut external_stop)) => {
                        let _ = (&mut external_stop).await;
                    }
                    (None, None) => {}
                }
                let _ = stop_tx.send(());
            })),
            None,
        )
    } else {
        (None, Some(stop_tx))
    };

    let monitor_task = tokio::spawn(async move {
        crate::session::run_monitor(monitor_config, None, Some(event_tx), Some(stop_rx)).await
    });

    loop {
        let event = tokio::select! {
            _ = research_deadline_tick.tick(), if _research_server.is_some() => {
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                research_coordinator.on_deadline(now);
                schedule_research_recovery(&mut research_coordinator, &runtime, now, &recovery_tx);
                continue;
            }
            event = event_rx.recv() => match event {
                Some(event) => event,
                None => break,
            },
            Some(context) = host_context_rx.recv() => {
                if let Err(error) = &context {
                    log::info!("research host context is blocked code={}", error.code);
                }
                research_coordinator.set_host_context(context);
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                schedule_research_recovery(&mut research_coordinator, &runtime, now, &recovery_tx);
                continue;
            }
            Some(result) = recovery_rx.recv() => {
                research_coordinator.set_mutable_action_present(
                    runtime.has_active_experiment()
                        || runtime.has_kept_actions()
                        || runtime.controller_state().active_experiment.is_some(),
                );
                research_coordinator.finish_recovery(result);
                if research_coordinator.host_refresh_complete()
                    && let Some(task) = host_context_refresh.take()
                {
                    task.abort();
                }
                continue;
            }
            Some(request) = research_rx.recv() => {
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                research_coordinator.set_mutable_action_present(
                    runtime.has_active_experiment()
                        || runtime.has_kept_actions()
                        || runtime.controller_state().active_experiment.is_some(),
                );
                research_coordinator.handle(request, now);
                // Admission freezes these facts. Future launch and block code must re-attest
                // its own context; repeatedly hashing the game here cannot supply that proof.
                if research_coordinator.host_refresh_complete()
                    && let Some(task) = host_context_refresh.take()
                {
                    task.abort();
                }
                continue;
            }
        };
        runtime.research_campaign_active =
            _research_server.is_some() && research_coordinator.owns_action_slot();
        runtime.on_event(event)?;
        event_loop_iterations = event_loop_iterations.saturating_add(1);
        if event_loop_iterations.is_multiple_of(30)
            && let Some(handle) = worker_handle.as_mut()
            && !handle.is_alive()
        {
            handle.restart()?;
            if handle.restart_count()
                > runtime
                    .config()
                    .daemon_config
                    .autotune
                    .privileged_worker_restart_limit
            {
                anyhow::bail!("privileged_worker_crash_loop");
            }
        }
    }

    if let Some(task) = host_context_refresh {
        task.abort();
    }
    if let Some(stop_task) = stop_task {
        stop_task.abort();
        let _ = stop_task.await;
    }

    let monitor_result = monitor_task.await?;
    if let Some(handle) = worker_handle.as_mut() {
        let shutdown_poll = Duration::from_millis(
            runtime
                .config()
                .daemon_config
                .autotune
                .privileged_worker_shutdown_poll_ms,
        );
        handle.shutdown_gracefully(Duration::from_millis(3_000), shutdown_poll)?;
    }
    finish_autotune_controller_session(&mut runtime, monitor_result)
}

pub(super) fn schedule_research_recovery(
    coordinator: &mut super::research_control::ResearchCoordinator,
    runtime: &AutotuneRuntime,
    now: u64,
    sender: &mpsc::Sender<anyhow::Result<super::research_control::RecoveredCampaign>>,
) {
    coordinator.set_mutable_action_present(
        runtime.has_active_experiment()
            || runtime.has_kept_actions()
            || runtime.controller_state().active_experiment.is_some(),
    );
    let Some(job) = coordinator.prepare_recovery(now) else {
        return;
    };
    let sender = sender.clone();
    tokio::spawn(async move {
        let result = match tokio::task::spawn_blocking(move || job.run()).await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("research recovery task failed: {error}")),
        };
        let _ = sender.send(result).await;
    });
}

pub(crate) fn finish_autotune_controller_session(
    runtime: &mut AutotuneRuntime,
    monitor_result: anyhow::Result<String>,
) -> anyhow::Result<AutotuneControllerExit> {
    let reason = match monitor_result {
        Ok(reason) => reason,
        Err(err) => {
            if let Err(rollback_err) =
                runtime.rollback_on_stop("autotune controller session failed before clean shutdown")
            {
                return Err(anyhow::anyhow!(
                    "monitor failed with {err:#}; additionally failed to rollback active experiment: {rollback_err:#}"
                ));
            }
            return Err(err);
        }
    };

    if let Some(snapshot) =
        runtime.rollback_on_stop(&format!("autotune controller session stopped: {reason}"))?
    {
        log::info!(
            "autotune_controller_session_exit_rollback_complete phase={:?} active_experiment={}",
            snapshot.phase,
            snapshot.active_experiment.is_some()
        );
    }

    Ok(AutotuneControllerExit {
        reason,
        last_decision: runtime.last_decision().cloned(),
    })
}
