//! Remote autotune route handler and daemon-state transition tests.

use super::{support::*, *};
fn agent_autotune_temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-agent-autotune-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn decode_restore_response(
    response: axum::response::Response,
) -> (StatusCode, crate::remote::AutotuneRestoreResponse) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let restore = serde_json::from_slice(&body).unwrap();
    (status, restore)
}

mod start;

mod status;

mod policy_rejection;

mod limits;

mod restore;

mod stop;

mod config;
