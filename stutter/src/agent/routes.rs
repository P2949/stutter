//! Agent route construction boundary.

use std::sync::Arc;

use axum::Router;

use super::{AgentRateLimiter, AgentState};

#[allow(dead_code)] // Transitional route extraction point; run_agent still owns full route wiring during staged split.
pub(crate) fn build_agent_router(
    _state: Arc<AgentState>,
    _rate_limiter: Arc<AgentRateLimiter>,
) -> Router {
    Router::new()
}
