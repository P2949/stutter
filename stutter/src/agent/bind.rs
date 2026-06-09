//! Agent binding and startup wiring.

use super::*;

pub async fn run_agent(config: AgentConfig) -> anyhow::Result<()> {
    if config.unix_socket.is_none() && !is_local_bind(&config.bind) && !config.allow_unsafe_bind {
        anyhow::bail!(
            "refusing to bind stutter agent to non-loopback address {}; \
             pass --allow-unsafe-bind only on trusted networks",
            config.bind
        );
    }

    if !config.runs_dir.exists() {
        std::fs::create_dir_all(&config.runs_dir)?;
    }

    let startup_recovery =
        crate::autotune::startup_recovery::recover_controller_journal_on_startup(
            crate::autotune::startup_recovery::StartupRecoveryConfig {
                rollback_on_crash_recovery: config.rollback_on_crash_recovery,
                ..crate::autotune::startup_recovery::StartupRecoveryConfig::default()
            },
        )?;

    let initial_daemon_state = daemon_state_from_startup_recovery(&startup_recovery);

    match startup_recovery {
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
            experiment_id,
            action_id,
            affected_tasks,
            manual_restore_command,
        } => {
            log::info!(
                "autotune startup recovery: rolled back interrupted action experiment_id={} action_id={} affected_tasks={} manual_restore_command=\"{}\"",
                experiment_id,
                action_id,
                affected_tasks,
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Faulted {
            experiment_id,
            action_id,
            manual_restore_command,
            reason,
        } => {
            log::error!(
                "autotune startup recovery faulted: experiment_id={} action_id={} reason={}",
                experiment_id,
                action_id,
                reason
            );
            log::error!("manual restore command: {}", manual_restore_command);
            anyhow::bail!(
                "autotune startup recovery failed; controller is Faulted; manual restore command: {}",
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::RollbackDisabled {
            experiment_id,
            action_id,
            manual_restore_command,
        } => {
            log::warn!(
                "autotune startup recovery: rollback_on_crash_recovery=false; leaving applied journal in place experiment_id={} action_id={} manual_restore_command=\"{}\"",
                experiment_id,
                action_id,
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::ApplyingWithoutRollback {
            experiment_id,
            action_id,
        } => {
            log::warn!(
                "autotune startup recovery: found applying journal without rollback token experiment_id={} action_id={}; no automatic rollback attempted",
                experiment_id,
                action_id
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Clean => {}
    }

    if config.unix_socket.is_none() && !is_local_bind(&config.bind) {
        if config.bearer_token.is_none()
            && config.read_token.is_none()
            && config.apply_token.is_none()
        {
            log::warn!(
                "WARNING: stutter agent is listening on a non-loopback address without authentication because --allow-unsafe-bind was passed."
            );
        } else {
            log::warn!(
                "WARNING: stutter agent is listening on a non-loopback address. Authentication is enabled."
            );
        }
    }

    let auth = AgentAuth {
        bearer_token: config.bearer_token.clone(),
        read_token: config.read_token.clone(),
        apply_token: config.apply_token.clone(),
    };
    let auth_enabled = auth.any_token_configured();

    audit_agent_event(
        "remote-agent-listen",
        true,
        0,
        agent_listen_audit_message(&config, auth_enabled),
    );

    let listen_bind = config.bind;
    let listen_unix_socket = config.unix_socket.clone();
    let state = Arc::new(AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(initial_daemon_state),
        runs_dir: config.runs_dir,
        bind: listen_bind,
        unix_socket: listen_unix_socket.clone(),
        auth,
        autotune_limits: config.autotune_limits,
        health_thresholds: config.health_thresholds,
        limits: AgentLimits {
            max_duration_seconds: config.max_duration_seconds,
            max_targets: config.max_targets,
            max_concurrent_recordings: config.max_concurrent_recordings,
        },
    });

    let request_rate_limiter = Arc::new(AgentRateLimiter::default());
    let app = routes::build_agent_router(state, request_rate_limiter);

    if let Some(path) = listen_unix_socket {
        log::info!("stutter agent listening on unix://{}", path.display());
        if config.max_unix_connections == DEFAULT_AGENT_UNIX_CONNECTION_LIMIT
            && config.unix_connection_timeout == DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT
        {
            server::serve_unix_socket(path, app).await?;
        } else {
            server::serve_unix_socket_with_limits(
                path,
                app,
                config.max_unix_connections,
                config.unix_connection_timeout,
            )
            .await?;
        }
    } else {
        log::info!("stutter agent listening on http://{}", listen_bind);
        let listener = tokio::net::TcpListener::bind(listen_bind).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

pub(crate) fn agent_listen_audit_message(config: &AgentConfig, auth_enabled: bool) -> String {
    match config.unix_socket.as_ref() {
        Some(path) => format!(
            "bind=unix:{} auth_enabled={} allow_unsafe_bind={} runs_dir={} max_unix_connections={} unix_connection_timeout_ms={}",
            path.display(),
            auth_enabled,
            config.allow_unsafe_bind,
            config.runs_dir.display(),
            config.max_unix_connections,
            config.unix_connection_timeout.as_millis()
        ),
        None => format!(
            "bind={} auth_enabled={} allow_unsafe_bind={} runs_dir={}",
            config.bind,
            auth_enabled,
            config.allow_unsafe_bind,
            config.runs_dir.display()
        ),
    }
}

pub(crate) fn is_local_bind(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}
