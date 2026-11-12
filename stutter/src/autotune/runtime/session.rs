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
    let mut research_coordinator = Some(super::research_control::ResearchCoordinator::new(
        runtime
            .config()
            .daemon_config
            .autotune
            .research_campaign_dir
            .clone(),
        None,
        "BlockedHostIdentity",
    ));
    let mut research_work: Option<tokio::task::JoinHandle<_>> = None;
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

    let controller_result: anyhow::Result<()> = async {
    loop {
        let event = tokio::select! {
            _ = research_deadline_tick.tick(), if _research_server.is_some() && research_coordinator.is_some() => {
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                let coordinator = research_coordinator.as_mut().ok_or_else(|| anyhow::anyhow!("research coordinator unavailable"))?;
                if coordinator.deadline_due(now) {
                    let coordinator = research_coordinator.take().ok_or_else(|| anyhow::anyhow!("research coordinator unavailable"))?;
                    research_work = Some(spawn_research_work(&mut runtime, coordinator, move |coordinator| {
                        coordinator.on_deadline(now);
                    }));
                } else {
                    schedule_research_recovery(coordinator, &runtime, now, &recovery_tx);
                }
                continue;
            }
            event = event_rx.recv() => match event {
                Some(event) => event,
                None => break Ok(()),
            },
            Some(context) = host_context_rx.recv(), if research_coordinator.is_some() => {
                if let Err(error) = &context {
                    log::info!("research host context is blocked code={}", error.code);
                }
                let coordinator = research_coordinator.as_mut().ok_or_else(|| anyhow::anyhow!("research coordinator unavailable"))?;
                coordinator.set_host_context(context);
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                schedule_research_recovery(coordinator, &runtime, now, &recovery_tx);
                continue;
            }
            Some(result) = recovery_rx.recv(), if research_coordinator.is_some() => {
                let coordinator = research_coordinator.as_mut().ok_or_else(|| anyhow::anyhow!("research coordinator unavailable"))?;
                coordinator.set_mutable_action_present(
                    runtime.has_active_experiment()
                        || runtime.has_kept_actions()
                        || runtime.controller_state().active_experiment.is_some(),
                );
                coordinator.finish_recovery(result);
                if coordinator.host_refresh_complete()
                    && let Some(task) = host_context_refresh.take()
                {
                    task.abort();
                }
                continue;
            }
            Some(request) = research_rx.recv(), if research_coordinator.is_some() => {
                let now = (crate::audit::unix_nanos_now() / 1_000_000_000)
                    .min(u128::from(u64::MAX)) as u64;
                let mut coordinator = research_coordinator.take().ok_or_else(|| anyhow::anyhow!("research coordinator unavailable"))?;
                coordinator.set_mutable_action_present(
                    runtime.has_active_experiment()
                        || runtime.has_kept_actions()
                        || runtime.controller_state().active_experiment.is_some(),
                );
                // Reserve before dispatch: admission and durable writes run off the monitor
                // loop, but no normal action can start while their result is unresolved.
                research_work = Some(spawn_research_work(&mut runtime, coordinator, move |coordinator| {
                    coordinator.handle(request, now);
                }));
                continue;
            }
            Some(result) = async {
                match research_work.as_mut() {
                    Some(work) => Some(work.await),
                    None => None,
                }
            }, if research_work.is_some() => {
                research_work = None;
                let coordinator = result.map_err(|error| anyhow::anyhow!("research management task failed: {error}"))?;
                // Admission freezes these facts. Future launch and block code must re-attest
                // its own context; repeatedly hashing the game here cannot supply that proof.
                if coordinator.host_refresh_complete()
                    && let Some(task) = host_context_refresh.take()
                {
                    task.abort();
                }
                research_coordinator = Some(coordinator);
                continue;
            }
        };
        runtime.research_campaign_active = _research_server.is_some()
            && research_coordinator.as_ref().is_none_or(|coordinator| coordinator.owns_action_slot());
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
    }.await;

    let management_result = if let Some(work) = research_work {
        work.await.map(|_| ()).map_err(|error| {
            anyhow::anyhow!("research management task failed during shutdown: {error}")
        })
    } else {
        Ok(())
    };
    if let Some(task) = host_context_refresh {
        task.abort();
    }
    if let Some(stop_task) = stop_task {
        stop_task.abort();
        let _ = stop_task.await;
    }

    if let Err(error) = controller_result.and(management_result) {
        monitor_task.abort();
        let _ = monitor_task.await;
        return finish_autotune_controller_session(&mut runtime, Err(error));
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

fn spawn_research_work(
    runtime: &mut AutotuneRuntime,
    mut coordinator: super::research_control::ResearchCoordinator,
    work: impl FnOnce(&mut super::research_control::ResearchCoordinator) + Send + 'static,
) -> tokio::task::JoinHandle<super::research_control::ResearchCoordinator> {
    runtime.research_campaign_active = true;
    tokio::task::spawn_blocking(move || {
        work(&mut coordinator);
        coordinator
    })
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

#[cfg(test)]
mod research_management_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn slow_management_keeps_the_slot_reserved_without_blocking_the_executor() {
        let mut runtime = AutotuneRuntime::new(AutotuneRuntimeConfig::observe(None, None, None));
        let coordinator = super::super::research_control::ResearchCoordinator::new(
            None,
            None,
            "BlockedHostIdentity",
        );
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let work = spawn_research_work(&mut runtime, coordinator, move |_| {
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(runtime.research_campaign_active);
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await
        .unwrap();
        assert!(!work.is_finished());
        release_tx.send(()).unwrap();
        let coordinator = work.await.unwrap();
        assert!(!coordinator.owns_action_slot());
    }
}
