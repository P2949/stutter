//! Remote autotune history endpoint.

use super::*;

pub(crate) async fn autotune_history_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let path = crate::autotune::history::default_autotune_history_path();
    let events = match crate::autotune::history::read_autotune_history_events(&path) {
        Ok(events) => events,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to read autotune history: {err:#}"),
                }),
            )
                .into_response();
        }
    };

    let values = events
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>();

    let events = match values {
        Ok(values) => values,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to encode autotune history: {err}"),
                }),
            )
                .into_response();
        }
    };

    Json(AutotuneHistoryResponse {
        path: path.display().to_string(),
        events,
    })
    .into_response()
}
