//! Agent server startup boundary.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

pub(crate) async fn serve_unix_socket(path: PathBuf, app: Router) -> anyhow::Result<()> {
    serve_unix_socket_with_limits(
        path,
        app,
        DEFAULT_AGENT_UNIX_CONNECTION_LIMIT,
        DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT,
    )
    .await
}

pub(crate) async fn serve_unix_socket_with_limits(
    path: PathBuf,
    app: Router,
    max_connections: usize,
    connection_timeout: Duration,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        max_connections > 0,
        "agent unix socket max_connections must be greater than zero"
    );
    anyhow::ensure!(
        !connection_timeout.is_zero(),
        "agent unix socket connection_timeout must be greater than zero"
    );
    prepare_unix_socket_path(&path)?;
    let listener = tokio::net::UnixListener::bind(&path)
        .with_context(|| format!("failed to bind agent unix socket {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "failed to set permissions on agent socket {}",
            path.display()
        )
    })?;

    let connection_permits = Arc::new(Semaphore::new(max_connections));
    let active_connections = Arc::new(AtomicUsize::new(0));
    let rejected_connections = Arc::new(AtomicUsize::new(0));
    let connection_errors = Arc::new(AtomicUsize::new(0));
    let mut make_service = app.into_make_service();
    loop {
        let (socket, _) = listener
            .accept()
            .await
            .with_context(|| format!("failed to accept connection on {}", path.display()))?;
        let permit = match connection_permits.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let rejected = rejected_connections.fetch_add(1, Ordering::Relaxed) + 1;
                log::warn!(
                    "agent_unix_connection_limit_reached max_connections={max_connections} rejected_connections={rejected}"
                );
                drop(socket);
                continue;
            }
        };
        let active = active_connections.fetch_add(1, Ordering::Relaxed) + 1;
        log::debug!(
            "agent_unix_connection_accepted active_connections={active} max_connections={max_connections}"
        );
        let socket = TokioIo::new(socket);

        let tower_service = make_service
            .call(())
            .await
            .unwrap_or_else(|err| match err {})
            .map_request(|request: Request<Incoming>| request.map(Body::new));
        let hyper_service = TowerToHyperService::new(tower_service);

        let active_connections = active_connections.clone();
        let connection_errors = connection_errors.clone();
        // Each accepted socket owns its Hyper connection task; shared counters are atomics and the
        // semaphore permit bounds total concurrency until the task exits.
        tokio::spawn(async move {
            let _permit = permit;
            let builder = HyperConnectionBuilder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(socket, hyper_service);
            match tokio::time::timeout(connection_timeout, connection).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    let errors = connection_errors.fetch_add(1, Ordering::Relaxed) + 1;
                    log::debug!("agent_unix_connection_failed errors={errors} err={err:#}");
                }
                Err(_) => {
                    log::warn!(
                        "agent_unix_connection_timeout timeout_ms={}",
                        connection_timeout.as_millis()
                    );
                }
            }
            let active = active_connections.fetch_sub(1, Ordering::Relaxed) - 1;
            log::debug!("agent_unix_connection_closed active_connections={active}");
        });
    }
}

pub(crate) async fn agent_request_guard(
    State(rate_limiter): State<Arc<AgentRateLimiter>>,
    request: Request,
    next: Next,
) -> Response {
    let request_id = agent_request_id(&request);
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();

    if !rate_limiter.accept(Instant::now()).await {
        audit_agent_event(
            "remote-agent-request",
            false,
            0,
            format!("request_id={request_id} method={method} path={path} status=429"),
        );
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    let response = next.run(request).await;
    let status = response.status();
    audit_agent_event(
        "remote-agent-request",
        status.is_success(),
        0,
        format!("request_id={request_id} method={method} path={path} status={status}"),
    );
    response
}

pub(crate) fn agent_request_id(request: &Request) -> String {
    if let Some(value) = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128)
    {
        return value.to_owned();
    }

    format!("generated-{}", crate::audit::unix_nanos_now())
}

pub(crate) fn prepare_unix_socket_path(path: &StdPath) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create agent unix socket directory {}",
                parent.display()
            )
        })?;
    }

    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect existing agent socket {}", path.display()))?;
    if !metadata.file_type().is_socket() {
        anyhow::bail!(
            "refusing to replace non-socket path for agent unix socket {}",
            path.display()
        );
    }

    std::fs::remove_file(path)
        .with_context(|| format!("failed to remove stale agent socket {}", path.display()))?;
    Ok(())
}
