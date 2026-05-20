//! Alert channel setup for monitor sessions.

use std::time::Duration;

use log::warn;

use crate::config::model::MonitorConfig;

pub(crate) struct AlertRuntime {
    pub(crate) sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
}

impl AlertRuntime {
    pub(crate) fn begin(config: &MonitorConfig) -> Self {
        let sender = if config.alerts.threshold_ns.is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let webhook_url = config.alerts.webhook_url.clone();
            let webhook_client = webhook_url.as_ref().map(|_| {
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
            });

            tokio::spawn(async move {
                while let Some(payload) = rx.recv().await {
                    if let Err(err) = crate::alert::send_desktop_alert(&payload).await {
                        warn!("desktop_alert_failed err={err}");
                    }
                    if let Some(url) = &webhook_url {
                        match &webhook_client {
                            Some(Ok(client)) => {
                                if let Err(err) = crate::alert::send_webhook_alert_with_client(
                                    client, url, &payload,
                                )
                                .await
                                {
                                    warn!("webhook_alert_failed url={url} err={err}");
                                }
                            }
                            Some(Err(err)) => {
                                warn!(
                                    "webhook_alert_failed url={url} err=failed to build HTTP client: {err}"
                                );
                            }
                            None => {}
                        }
                    }
                }
            });

            Some(tx)
        } else {
            None
        };

        Self { sender }
    }
}
