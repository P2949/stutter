use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cli::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteMonitorRequest {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub exclude_tree_pids: Vec<u32>,
    pub duration_seconds: Option<u64>,

    pub spike_us: Option<u64>,
    pub summary_ms: Option<u64>,

    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,

    pub hwmon: bool,
    pub cpu_freq: bool,
    pub faults: bool,
    pub stat_wait: bool,
    pub block_io: bool,

    pub irq_latency: bool,
    pub irqs: Vec<u32>,

    pub record: bool,
    pub run_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRecordResponse {
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopRecordResponse {
    pub run_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordStatusResponse {
    pub active: bool,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunsResponse {
    pub runs: Vec<String>,
}

pub fn request_from_monitor_config(config: &Config) -> anyhow::Result<RemoteMonitorRequest> {
    Ok(RemoteMonitorRequest {
        target_pids: config.target_pids.clone(),
        tree_pids: config.tree_pids.clone(),
        exclude_tree_pids: config.exclude_tree_pids.clone(),
        duration_seconds: config.max_duration.map(|d| d.as_secs()),
        spike_us: Some(config.spike_threshold_ns / 1000),
        summary_ms: Some(config.summary_period_ms),
        include_comm: config
            .task_filters
            .include_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        exclude_comm: config
            .task_filters
            .exclude_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        hwmon: config.hwmon,
        cpu_freq: config.cpu_freq,
        faults: config.faults,
        stat_wait: config.stat_wait,
        block_io: config.block_io,
        irq_latency: config.irq_latency,
        irqs: config.irqs.clone(),
        record: config.recording.is_some(),
        run_name: config.recording.as_ref().and_then(|r| r.run_name.clone()),
    })
}

pub async fn run_remote_monitor(
    endpoint: &str,
    request: RemoteMonitorRequest,
) -> anyhow::Result<()> {
    let base = endpoint.trim_end_matches('/');
    let client = reqwest::Client::new();

    let start_url = format!("{base}/record/start");
    log::info!("sending remote start request to {start_url}");

    let start: StartRecordResponse = client
        .post(&start_url)
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
            let _ = client.post(format!("{base}/record/stop")).send().await;
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

    let stop: StopRecordResponse = client
        .post(format!("{base}/record/stop"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!("remote recording stopped: run_id={:?}", stop.run_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_monitor_request_round_trips_json() {
        let request = RemoteMonitorRequest {
            target_pids: vec![1234],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(5),
            spike_us: Some(1000),
            summary_ms: Some(500),
            include_comm: vec!["Game".to_string()],
            exclude_comm: vec![],
            hwmon: true,
            cpu_freq: true,
            faults: true,
            stat_wait: true,
            block_io: false,
            irq_latency: false,
            irqs: vec![],
            record: true,
            run_name: Some("test".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: RemoteMonitorRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.target_pids, vec![1234]);
        assert_eq!(decoded.duration_seconds, Some(5));
        assert!(decoded.hwmon);
    }
}
