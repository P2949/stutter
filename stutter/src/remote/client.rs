use std::time::Duration;

use super::{
    monitor::RemoteMonitorRequest,
    responses::{StartRecordResponse, StopRecordResponse},
};

pub async fn run_remote_monitor(
    endpoint: &str,
    request: RemoteMonitorRequest,
) -> anyhow::Result<()> {
    let base = endpoint.trim_end_matches('/');
    let client = reqwest::Client::new();

    let start_url = format!("{base}/record/start");
    log::info!("sending remote start request to {start_url}");

    let start: StartRecordResponse = apply_auth(client.post(&start_url))
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!("remote recording started: run_id={}", start.run_id);

    let stop_handle = tokio::spawn({
        let client = client.clone();
        let base = base.to_owned();
        async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                log::error!("failed to wait for ctrl-c: {e}");
            }
            log::info!("ctrl-c detected, sending remote stop request");
            let _ = apply_auth(client.post(format!("{base}/record/stop")))
                .send()
                .await;
        }
    });

    if let Some(seconds) = request.duration_seconds {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => {
                log::info!("duration reached, sending remote stop request");
            }
            _ = stop_handle => {
                return Ok(());
            }
        }
    } else {
        stop_handle.await?;
        return Ok(());
    }

    let stop: StopRecordResponse = apply_auth(client.post(format!("{base}/record/stop")))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!("remote recording stopped: run_id={:?}", stop.run_id);

    Ok(())
}

fn maybe_bearer_token_from_env() -> Option<String> {
    std::env::var("STUTTER_AGENT_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn apply_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(token) = maybe_bearer_token_from_env() {
        request.bearer_auth(token)
    } else {
        request
    }
}
