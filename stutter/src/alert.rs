use std::time::Duration;

use serde::Serialize;
use stutter_common::SchedulerEvent;

use crate::{
    metrics::{self, format_latency},
    process_tree::TaskClass,
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AlertPayload {
    pub title: String,
    pub message: String,
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub comm: String,
    pub process_pid: Option<u32>,
    pub process_comm: String,
    pub latency_ns: u64,
    pub latency_ms: u64,
    pub cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub elapsed_ms: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_ops: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_state: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scx_enable_seq: Option<String>,
}

impl AlertPayload {
    pub fn from_task_stats(
        stats: &metrics::TaskStats,
        event: &SchedulerEvent,
        elapsed_ms: u64,
        scx_ops: Option<&str>,
        scx_state: Option<&str>,
        scx_enable_seq: Option<&str>,
    ) -> Self {
        let latency_ms = event.latency_ns / 1_000_000;
        let title = "stutter latency alert".to_owned();
        let message = format!(
            "task={} comm={} latency={} cpu={} process_pid={:?} process_comm={}",
            event.tid,
            stats.comm,
            format_latency(event.latency_ns),
            event.cpu,
            stats.process_pid,
            stats.process_comm
        );

        Self {
            title,
            message,
            task: event.tid,
            active: stats.active,
            class: stats.class,
            comm: stats.comm.clone(),
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.to_string(),
            latency_ns: event.latency_ns,
            latency_ms,
            cpu: event.cpu,
            prio: event.prio,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            elapsed_ms,
            scx_ops: scx_ops.map(str::to_owned),
            scx_state: scx_state.map(str::to_owned),
            scx_enable_seq: scx_enable_seq.map(str::to_owned),
        }
    }
}

/// Sends a desktop notification using `notify-send`.
///
/// This remains a best-effort local desktop integration. It intentionally
/// uses an external command and may add system noise, so alert failures are
/// logged by the caller and do not stop monitoring.
pub async fn send_desktop_alert(payload: &AlertPayload) -> Result<(), String> {
    let mut child = tokio::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            payload.title.as_str(),
            payload.message.as_str(),
        ])
        .spawn()
        .map_err(|err| format!("failed to spawn notify-send: {err}"))?;

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .map_err(|_| "notify-send timed out after 10 seconds".to_owned())?
        .map_err(|err| format!("failed to wait for notify-send: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("notify-send exited with {status}"))
    }
}

fn validate_webhook_url(url: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url).map_err(|err| format!("invalid webhook URL: {err}"))?;

    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(format!(
            "unsupported webhook URL scheme `{other}`; only http and https are allowed"
        )),
    }
}

pub async fn send_webhook_alert_with_client(
    client: &reqwest::Client,
    url: &str,
    payload: &AlertPayload,
) -> Result<(), String> {
    let parsed_url = validate_webhook_url(url)?;
    let response = client
        .post(parsed_url)
        .json(payload)
        .send()
        .await
        .map_err(|err| format!("failed to send webhook alert: {err}"))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read response body>".to_owned());

        Err(format!(
            "webhook alert failed with HTTP status {status}: {body}"
        ))
    }
}

#[allow(dead_code)]
pub async fn send_webhook_alert(url: &str, payload: &AlertPayload) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| format!("failed to build webhook HTTP client: {err}"))?;

    send_webhook_alert_with_client(&client, url, payload).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_webhook_url_allows_http_and_https() {
        assert!(validate_webhook_url("http://example.com/hook").is_ok());
        assert!(validate_webhook_url("https://example.com/hook").is_ok());
    }

    #[test]
    fn validate_webhook_url_rejects_non_http_schemes() {
        let err = validate_webhook_url("file:///tmp/hook").unwrap_err();
        assert!(err.contains("only http and https are allowed"));

        let err = validate_webhook_url("ftp://example.com/hook").unwrap_err();
        assert!(err.contains("only http and https are allowed"));
    }

    #[test]
    fn validate_webhook_url_rejects_relative_urls() {
        assert!(validate_webhook_url("example.com/hook").is_err());
        assert!(validate_webhook_url("/local/path").is_err());
    }
}
