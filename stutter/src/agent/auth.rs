//! Agent authentication contracts.

use super::*;

#[derive(Clone, Debug, Default)]
pub struct AgentAuth {
    pub bearer_token: Option<String>,
    pub read_token: Option<String>,
    pub apply_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentAuthScope {
    Read,
    Apply,
}

impl AgentAuth {
    pub(crate) fn any_token_configured(&self) -> bool {
        self.bearer_token.is_some() || self.read_token.is_some() || self.apply_token.is_some()
    }

    pub(crate) fn apply_token_configured(&self) -> bool {
        self.bearer_token.is_some() || self.apply_token.is_some()
    }
}

pub(crate) fn authorize(headers: &HeaderMap, auth: &AgentAuth) -> Result<(), StatusCode> {
    authorize_with_scope(headers, auth, AgentAuthScope::Read)
}

pub(crate) fn authorize_apply(headers: &HeaderMap, auth: &AgentAuth) -> Result<(), StatusCode> {
    authorize_with_scope(headers, auth, AgentAuthScope::Apply)
}

#[cfg(test)]
pub(crate) fn authorize_state_change(
    headers: &HeaderMap,
    state: &AgentState,
) -> Result<(), StatusCode> {
    authorize_agent_privileged_operation(headers, state, PrivilegedOperation::ControlDaemon)
}

pub(crate) fn authorize_remote_autotune_start(
    headers: &HeaderMap,
    state: &AgentState,
) -> Result<(), StatusCode> {
    if state.unix_socket.is_none() && !state.auth.apply_token_configured() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    authorize_apply(headers, &state.auth)
}

pub(crate) fn authorize_agent_privileged_operation(
    headers: &HeaderMap,
    state: &AgentState,
    operation: PrivilegedOperation,
) -> Result<(), StatusCode> {
    let transport = agent_privilege_transport(state);
    let socket_authorized =
        matches!(transport, PrivilegeTransport::UnixSocket) && !state.auth.any_token_configured();

    if !socket_authorized
        && operation.requires_apply_authorization()
        && !state.auth.apply_token_configured()
    {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let apply_result = if socket_authorized {
        Ok(())
    } else {
        authorize_apply(headers, &state.auth)
    };

    if operation.requires_apply_authorization()
        && let Err(status) = apply_result
    {
        return Err(status);
    }

    let request = PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation,
        transport,
        authenticated: socket_authorized || authorize(headers, &state.auth).is_ok(),
        apply_authorized: socket_authorized || apply_result.is_ok(),
    };
    let decision = PrivilegeCommandAllowlist.check(&request);
    if decision.allowed {
        Ok(())
    } else {
        Err(status_for_privilege_decision(&decision.reason_code))
    }
}

pub(crate) fn agent_privilege_transport(state: &AgentState) -> PrivilegeTransport {
    if state.unix_socket.is_some() {
        PrivilegeTransport::UnixSocket
    } else if state.bind.ip().is_loopback() {
        PrivilegeTransport::LoopbackTcp
    } else {
        PrivilegeTransport::RemoteTcp
    }
}

fn status_for_privilege_decision(reason_code: &str) -> StatusCode {
    match reason_code {
        "missing_authentication" | "missing_apply_authorization" => StatusCode::UNAUTHORIZED,
        _ => StatusCode::FORBIDDEN,
    }
}

fn authorize_with_scope(
    headers: &HeaderMap,
    auth: &AgentAuth,
    scope: AgentAuthScope,
) -> Result<(), StatusCode> {
    if !auth.any_token_configured() {
        return Ok(());
    };

    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Ok(value) = header.to_str() else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Some(token) = value.strip_prefix("Bearer ") else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let legacy_match = token_matches(token, auth.bearer_token.as_deref());
    let apply_match = token_matches(token, auth.apply_token.as_deref());
    let read_match = token_matches(token, auth.read_token.as_deref());

    match scope {
        AgentAuthScope::Read if legacy_match || apply_match || read_match => Ok(()),
        AgentAuthScope::Apply if legacy_match || apply_match => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

fn token_matches(token: &str, expected: Option<&str>) -> bool {
    expected.is_some_and(|expected| constant_time_eq(token.as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub(crate) fn agent_state_is_local(state: &AgentState) -> bool {
    state.unix_socket.is_some() || is_local_bind(&state.bind)
}

pub(crate) fn remote_access_status(state: &AgentState) -> RemoteAccessStatus {
    RemoteAccessStatus {
        remote_transport: remote_transport_label(state).to_owned(),
        remote_auth_configured: state.auth.any_token_configured(),
        remote_apply_enabled: remote_apply_enabled(state),
    }
}

fn remote_transport_label(state: &AgentState) -> &'static str {
    match agent_privilege_transport(state) {
        PrivilegeTransport::UnixSocket => "unix_socket",
        PrivilegeTransport::LoopbackTcp | PrivilegeTransport::LocalCli => "loopback_tcp",
        PrivilegeTransport::RemoteTcp => "unsafe_tcp",
    }
}

fn remote_apply_enabled(state: &AgentState) -> bool {
    agent_state_is_local(state)
        && state.auth.apply_token_configured()
        && autotune::remote_mode_supported(&state.autotune_limits, DaemonMode::ApplyLowRisk)
}
