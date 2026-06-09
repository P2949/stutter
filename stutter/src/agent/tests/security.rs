//! Agent binding, authentication, privilege, and title-capture security tests.

use tower::ServiceExt as _;

use super::{support::*, *};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RouteAuthExpectation {
    route: &'static str,
    request_path: &'static str,
    method: &'static str,
    body: RouteAuthBody,
    requires_auth: bool,
    rejects_non_loopback_apply: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteAuthBody {
    Empty,
    RecordStart,
    AutotuneStartApplyLowRisk,
}

const ROUTE_AUTH_EXPECTATIONS: &[RouteAuthExpectation] = &[
    route_auth_row(
        "GET",
        "/health",
        "/health",
        RouteAuthBody::Empty,
        false,
        false,
    ),
    route_auth_row(
        "GET",
        "/version",
        "/version",
        RouteAuthBody::Empty,
        false,
        false,
    ),
    route_auth_row(
        "GET",
        "/capabilities",
        "/capabilities",
        RouteAuthBody::Empty,
        false,
        false,
    ),
    route_auth_row(
        "POST",
        "/record/start",
        "/record/start",
        RouteAuthBody::RecordStart,
        true,
        true,
    ),
    route_auth_row(
        "POST",
        "/record/stop",
        "/record/stop",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row(
        "GET",
        "/record/status",
        "/record/status",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/autotune/status",
        "/autotune/status",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "POST",
        "/autotune/start",
        "/autotune/start",
        RouteAuthBody::AutotuneStartApplyLowRisk,
        true,
        true,
    ),
    route_auth_row(
        "POST",
        "/autotune/stop",
        "/autotune/stop",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row(
        "POST",
        "/autotune/restore",
        "/autotune/restore",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row(
        "GET",
        "/autotune/history",
        "/autotune/history",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/autotune/config",
        "/autotune/config",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/daemon/status",
        "/daemon/status",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/daemon/health",
        "/daemon/health",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/daemon/policy",
        "/daemon/policy",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/daemon/explain",
        "/daemon/explain",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "POST",
        "/daemon/pause",
        "/daemon/pause",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row(
        "POST",
        "/daemon/resume",
        "/daemon/resume",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row(
        "POST",
        "/daemon/restore",
        "/daemon/restore",
        RouteAuthBody::Empty,
        true,
        true,
    ),
    route_auth_row("GET", "/runs", "/runs", RouteAuthBody::Empty, true, false),
    route_auth_row(
        "GET",
        "/runs/:id/session.json",
        "/runs/run-123/session.json",
        RouteAuthBody::Empty,
        true,
        false,
    ),
    route_auth_row(
        "GET",
        "/runs/:id/artifact/:name",
        "/runs/run-123/artifact/session.json",
        RouteAuthBody::Empty,
        true,
        false,
    ),
];

const fn route_auth_row(
    method: &'static str,
    route: &'static str,
    request_path: &'static str,
    body: RouteAuthBody,
    requires_auth: bool,
    rejects_non_loopback_apply: bool,
) -> RouteAuthExpectation {
    RouteAuthExpectation {
        route,
        request_path,
        method,
        body,
        requires_auth,
        rejects_non_loopback_apply,
    }
}

#[test]
fn validate_id_rejects_path_traversal() {
    assert!(validate_id("../evil").is_err());
    assert!(validate_id("foo/bar").is_err());
    assert!(validate_id("run-123").is_ok());
}

#[test]
fn artifact_allowlist_rejects_unknown_file() {
    assert!(validate_artifact_name("shadow").is_err());
    assert!(validate_artifact_name("session.json.bak").is_err());
}

#[test]
fn artifact_allowlist_rejects_path_traversal() {
    assert!(validate_artifact_name("../session.json").is_err());
}

#[test]
fn artifact_allowlist_accepts_known_artifacts() {
    assert!(validate_artifact_name("session.json").is_ok());
    assert!(validate_artifact_name("metadata.json").is_ok());
    assert!(validate_artifact_name("spike_events.json").is_ok());
}

#[test]
fn route_auth_matrix_covers_capability_routes() {
    let state = test_agent_state_custom("127.0.0.1:0".parse().unwrap(), Some("secret".to_owned()));
    let supported_routes = routes::capabilities_response(&state).supported_routes;
    let matrix_routes = ROUTE_AUTH_EXPECTATIONS
        .iter()
        .map(|row| row.route.to_owned())
        .collect::<Vec<_>>();

    assert_eq!(matrix_routes, supported_routes);
}

#[tokio::test]
async fn route_auth_matrix_rejects_missing_auth_where_required() {
    for row in ROUTE_AUTH_EXPECTATIONS {
        let mut state =
            test_agent_state_custom("127.0.0.1:0".parse().unwrap(), Some("secret".to_owned()));
        state.unix_socket = None;

        let status = route_auth_status(*row, state, None).await;
        if row.requires_auth {
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "route {} {} should require auth",
                row.method,
                row.route
            );
        } else {
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "route {} {} should be public",
                row.method,
                row.route
            );
            assert_ne!(
                status,
                StatusCode::FORBIDDEN,
                "route {} {} should be public",
                row.method,
                row.route
            );
        }
    }
}

#[tokio::test]
async fn route_auth_matrix_rejects_non_loopback_apply_routes() {
    for row in ROUTE_AUTH_EXPECTATIONS
        .iter()
        .copied()
        .filter(|row| row.rejects_non_loopback_apply)
    {
        let mut state =
            test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
        state.unix_socket = None;

        let status = route_auth_status(row, state, Some("secret")).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "route {} {} should reject non-loopback state changes",
            row.method,
            row.route
        );
    }
}

async fn route_auth_status(
    row: RouteAuthExpectation,
    state: AgentState,
    bearer_token: Option<&str>,
) -> StatusCode {
    let router = routes::build_agent_router(
        Arc::new(state),
        Arc::new(AgentRateLimiter::new(usize::MAX, Duration::from_secs(60))),
    );
    let mut builder = Request::builder().method(row.method).uri(row.request_path);
    if let Some(token) = bearer_token {
        builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if route_auth_body_is_json(row.body) {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
    }

    let response = router
        .oneshot(
            builder
                .body(route_auth_body(row.body))
                .expect("route auth test request should build"),
        )
        .await
        .expect("route auth test request should run");
    response.status()
}

fn route_auth_body_is_json(body: RouteAuthBody) -> bool {
    matches!(
        body,
        RouteAuthBody::RecordStart | RouteAuthBody::AutotuneStartApplyLowRisk
    )
}

fn route_auth_body(body: RouteAuthBody) -> Body {
    match body {
        RouteAuthBody::Empty => Body::empty(),
        RouteAuthBody::RecordStart => {
            Body::from(serde_json::to_vec(&minimal_remote_request()).unwrap())
        }
        RouteAuthBody::AutotuneStartApplyLowRisk => {
            Body::from(serde_json::to_vec(&autotune_request("apply-low-risk")).unwrap())
        }
    }
}

#[test]
fn local_bind_allowed_without_unsafe_flag() {
    let addr: SocketAddr = "127.0.0.1:9899".parse().unwrap();
    assert!(validate_agent_bind_policy(addr, false).is_ok());
}

#[test]
fn non_loopback_bind_rejected_without_unsafe_flag() {
    let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
    assert!(validate_agent_bind_policy(addr, false).is_err());
}

#[test]
fn non_loopback_bind_allowed_with_unsafe_flag() {
    let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
    assert!(validate_agent_bind_policy(addr, true).is_ok());
}

#[test]
fn unix_socket_state_counts_as_local() {
    let mut state = test_agent_state("0.0.0.0:9899".parse().unwrap(), None);
    assert!(!agent_state_is_local(&state));

    state.unix_socket = Some(PathBuf::from("/tmp/stutter-agent.sock"));
    assert!(agent_state_is_local(&state));
}

#[test]
fn listen_audit_message_reports_unix_socket() {
    let config = AgentConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        unix_socket: Some(PathBuf::from("/tmp/stutter-agent.sock")),
        runs_dir: PathBuf::from("/tmp/stutter-runs"),
        allow_unsafe_bind: false,
        bearer_token: None,
        read_token: None,
        apply_token: None,
        max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
        max_targets: DEFAULT_AGENT_MAX_TARGETS,
        max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
        max_unix_connections: DEFAULT_AGENT_UNIX_CONNECTION_LIMIT,
        unix_connection_timeout: DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT,
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: SystemHealthThresholds::default(),
        rollback_on_crash_recovery: true,
    };

    let message = agent_listen_audit_message(&config, false);
    assert!(message.contains("bind=unix:/tmp/stutter-agent.sock"));
    assert!(message.contains("auth_enabled=false"));
    assert!(message.contains("max_unix_connections=128"));
    assert!(message.contains("unix_connection_timeout_ms=60000"));
}

fn validate_agent_bind_policy(bind: SocketAddr, allow_unsafe_bind: bool) -> anyhow::Result<()> {
    if !is_local_bind(&bind) && !allow_unsafe_bind {
        anyhow::bail!("refusing to bind");
    }
    Ok(())
}

#[test]
fn normalize_bearer_token_trims_newline() {
    assert_eq!(
        normalize_bearer_token("  secret\n  ".to_owned()).unwrap(),
        Some("secret".to_owned())
    );
}

#[test]
fn normalize_bearer_token_rejects_empty() {
    assert!(normalize_bearer_token("  ".to_owned()).is_err());
}

#[test]
fn authorize_allows_without_configured_token() {
    let auth = AgentAuth::default();
    let headers = HeaderMap::new();
    assert!(authorize(&headers, &auth).is_ok());
}

#[test]
fn authorize_rejects_missing_bearer_token() {
    let auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    let headers = HeaderMap::new();
    assert_eq!(authorize(&headers, &auth), Err(StatusCode::UNAUTHORIZED));
}

#[test]
fn authorize_rejects_wrong_bearer_token() {
    let auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer wrong".parse().unwrap(),
    );
    assert_eq!(authorize(&headers, &auth), Err(StatusCode::FORBIDDEN));
}

#[test]
fn authorize_accepts_correct_bearer_token() {
    let auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer secret".parse().unwrap(),
    );
    assert!(authorize(&headers, &auth).is_ok());
}

#[test]
fn read_token_can_read_but_cannot_apply() {
    let auth = AgentAuth {
        read_token: Some("read-secret".to_owned()),
        apply_token: Some("apply-secret".to_owned()),
        ..AgentAuth::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer read-secret".parse().unwrap(),
    );

    assert!(authorize(&headers, &auth).is_ok());
    assert_eq!(authorize_apply(&headers, &auth), Err(StatusCode::FORBIDDEN));
}

#[test]
fn apply_token_can_read_and_apply() {
    let auth = AgentAuth {
        read_token: Some("read-secret".to_owned()),
        apply_token: Some("apply-secret".to_owned()),
        ..AgentAuth::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer apply-secret".parse().unwrap(),
    );

    assert!(authorize(&headers, &auth).is_ok());
    assert!(authorize_apply(&headers, &auth).is_ok());
}

#[test]
fn tcp_state_change_requires_apply_token() {
    let mut state = test_agent_state("127.0.0.1:9899".parse().unwrap(), None);
    state.unix_socket = None;

    assert_eq!(
        authorize_state_change(&HeaderMap::new(), &state),
        Err(StatusCode::UNAUTHORIZED)
    );
}

#[test]
fn unix_socket_state_change_allows_socket_credentials_without_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);

    assert!(state.unix_socket.is_some());
    assert!(authorize_state_change(&HeaderMap::new(), &state).is_ok());
}

#[test]
fn unix_socket_state_change_respects_configured_apply_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer secret".parse().unwrap(),
    );

    assert!(state.unix_socket.is_some());
    assert_eq!(
        authorize_state_change(&HeaderMap::new(), &state),
        Err(StatusCode::UNAUTHORIZED)
    );
    assert!(authorize_state_change(&headers, &state).is_ok());
}

#[test]
fn remote_tcp_privileged_operation_is_rejected_even_with_apply_token() {
    let state = test_agent_state("0.0.0.0:9899".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer secret".parse().unwrap(),
    );

    assert_eq!(
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::StartRecording),
        Err(StatusCode::FORBIDDEN)
    );
}

#[test]
fn loopback_tcp_privileged_operation_requires_apply_scope() {
    let mut state = test_agent_state("127.0.0.1:9899".parse().unwrap(), None);
    state.unix_socket = None;
    state.auth = AgentAuth {
        read_token: Some("read-secret".to_owned()),
        apply_token: Some("apply-secret".to_owned()),
        ..AgentAuth::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer read-secret".parse().unwrap(),
    );

    assert_eq!(
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::ControlDaemon),
        Err(StatusCode::FORBIDDEN)
    );
}

#[test]
fn agent_privilege_transport_prefers_unix_socket_when_configured() {
    let mut state = test_agent_state("0.0.0.0:9899".parse().unwrap(), None);
    state.unix_socket = Some(PathBuf::from("/tmp/stutter-agent-test.sock"));

    assert_eq!(
        agent_privilege_transport(&state),
        PrivilegeTransport::UnixSocket
    );
}

#[tokio::test]
async fn agent_rate_limiter_drops_until_window_expires() {
    let limiter = AgentRateLimiter::new(2, Duration::from_secs(10));
    let now = Instant::now();

    assert!(limiter.accept(now).await);
    assert!(limiter.accept(now + Duration::from_secs(1)).await);
    assert!(!limiter.accept(now + Duration::from_secs(2)).await);
    assert!(limiter.accept(now + Duration::from_secs(11)).await);
}
#[test]
fn foreground_title_capture_allowed_on_loopback_without_token() {
    let mut request = minimal_remote_request();
    request.foreground_include_title = true;
    let state = test_agent_state_custom("127.0.0.1:0".parse().unwrap(), None);
    let headers = HeaderMap::new();

    assert!(validate_foreground_title_capture_security(&request, &state, &headers).is_ok());
}

#[test]
fn foreground_title_capture_rejected_on_non_loopback_without_token() {
    let mut request = minimal_remote_request();
    request.foreground_include_title = true;
    let state = test_agent_state_custom("0.0.0.0:0".parse().unwrap(), None);
    let headers = HeaderMap::new();

    assert_eq!(
        validate_foreground_title_capture_security(&request, &state, &headers),
        Err(StatusCode::FORBIDDEN)
    );
}

#[test]
fn foreground_title_capture_rejected_on_non_loopback_with_valid_bearer_token() {
    let mut request = minimal_remote_request();
    request.foreground_include_title = true;
    let state = test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer secret".parse().unwrap(),
    );

    assert_eq!(
        validate_foreground_title_capture_security(&request, &state, &headers),
        Err(StatusCode::FORBIDDEN)
    );
}

#[test]
fn foreground_title_capture_rejected_on_non_loopback_with_invalid_bearer_token() {
    let mut request = minimal_remote_request();
    request.foreground_include_title = true;
    let state = test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer wrong".parse().unwrap(),
    );

    assert_eq!(
        validate_foreground_title_capture_security(&request, &state, &headers),
        Err(StatusCode::FORBIDDEN)
    );
}
