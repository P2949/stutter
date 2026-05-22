//! Remote autotune config endpoint.

use super::{
    policy::{remote_mode_supported, supported_remote_mode_labels},
    *,
};

pub(crate) async fn autotune_config_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    Json(AutotuneConfigResponse {
        default_mode: DaemonMode::Observe.as_str().to_owned(),
        supported_modes: supported_remote_mode_labels(&state.autotune_limits),
        apply_low_risk_remote_enabled: remote_mode_supported(
            &state.autotune_limits,
            DaemonMode::ApplyLowRisk,
        ),
        local_only_by_default: true,
        history_path: crate::autotune::history::default_autotune_history_path()
            .display()
            .to_string(),
        autotune_limits: state.autotune_limits.clone(),
        daemon_scope: "remote-agent".to_owned(),
        allow_system_wide_suggestions: state.autotune_limits.allow_system_wide_suggestions,
        allow_system_wide_apply: state.autotune_limits.allow_system_wide_apply,
        minimum_focus_confidence: 0.70,
        required_stable_focus_polls: 3,
    })
    .into_response()
}
