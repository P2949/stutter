//! Passive Wayland idle observation for an admitted desktop research campaign.
//!
//! This signal is not launch authorization: foreground ownership, Steam, session and attempt
//! safety still require independent checks. No input contents or window titles are collected.
mod connect;
mod protocol;

use serde::Serialize;
use tokio::sync::{mpsc, watch};

/// Owns the passive observer. A missing, stopped, or stale observer is always unknown.
pub(crate) struct DesktopIdleMonitor {
    target: Option<(u32, String, String)>,
    latest: std::sync::Arc<std::sync::Mutex<Option<(std::time::Instant, DesktopIdleObservation)>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl DesktopIdleMonitor {
    pub(crate) fn new() -> Self {
        Self {
            target: None,
            latest: std::sync::Arc::new(std::sync::Mutex::new(None)),
            task: None,
        }
    }

    pub(crate) fn set_target(&mut self, target: Option<(u32, String, String)>) {
        if self.target == target {
            return;
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.latest = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.target = target.clone();
        if let Some((uid, session, _intent_digest)) = target {
            let latest = self.latest.clone();
            self.task = Some(tokio::spawn(async move {
                loop {
                    let (tx, mut rx) = mpsc::channel(8);
                    let (_stop, stop_rx) = watch::channel(false);
                    let observer = observe(uid, session.clone(), 60_000, tx, stop_rx);
                    tokio::pin!(observer);
                    loop {
                        tokio::select! {
                            _ = &mut observer => break,
                            value = rx.recv() => match value {
                                Some(value) => if let Ok(mut last) = latest.lock() {
                                    *last = Some((std::time::Instant::now(), value));
                                } else { return; },
                                None => break,
                            },
                        }
                    }
                    if let Ok(mut last) = latest.lock() {
                        *last = None;
                    } else {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }));
        }
    }

    pub(crate) fn observation(&self) -> (std::time::Instant, DesktopIdleObservation) {
        self.latest
            .lock()
            .ok()
            .and_then(|last| *last)
            .filter(|(time, _)| time.elapsed() <= std::time::Duration::from_secs(3))
            .unwrap_or_else(|| (std::time::Instant::now(), DesktopIdleObservation::Unknown))
    }
}

impl Drop for DesktopIdleMonitor {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopIdleObservation {
    Unknown,
    Monitoring {
        inhibitor_idle: bool,
        input_idle: bool,
    },
}

/// Observe one independently authenticated compositor in the active local graphical session.
/// Dropping the future closes the connection. Any error withdraws the signal.
pub(crate) async fn observe(
    uid: u32,
    session: String,
    idle_millis: u32,
    updates: mpsc::Sender<DesktopIdleObservation>,
    stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1_000..=600_000).contains(&idle_millis),
        "invalid idle interval"
    );
    let compositor = connect::active_compositor(uid, &session).await?;
    let stream = compositor.stream.try_clone()?;
    let observer = protocol::observe(stream, idle_millis, updates, stop);
    tokio::pin!(observer);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        tokio::select! {
            result = &mut observer => return result,
            _ = interval.tick() => {
                tokio::time::timeout(std::time::Duration::from_secs(3), compositor.revalidate(uid, &session)).await??;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_stale_observations_are_unknown() {
        let monitor = DesktopIdleMonitor::new();
        assert_eq!(monitor.observation().1, DesktopIdleObservation::Unknown);
        let sample = DesktopIdleObservation::Monitoring {
            inhibitor_idle: true,
            input_idle: true,
        };
        let sampled_at = std::time::Instant::now();
        *monitor.latest.lock().unwrap() = Some((sampled_at, sample));
        assert_eq!(monitor.observation(), (sampled_at, sample));
        *monitor.latest.lock().unwrap() = Some((
            std::time::Instant::now() - std::time::Duration::from_secs(4),
            sample,
        ));
        assert_eq!(monitor.observation().1, DesktopIdleObservation::Unknown);
    }

    #[tokio::test]
    #[ignore = "requires the current user's active Hyprland Wayland session"]
    async fn observes_current_session_without_input_injection() {
        let uid = crate::syscall::geteuid();
        let session = super::super::host::active_wayland_session(uid).expect("active session");
        let (updates, mut received) = mpsc::channel(8);
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(observe(uid, session, 600_000, updates, stopped));
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(value) = received.recv().await {
                if matches!(value, DesktopIdleObservation::Monitoring { .. }) {
                    return value;
                }
            }
            panic!("observer stopped before setup");
        })
        .await
        .expect("observer setup timeout");
        assert!(matches!(result, DesktopIdleObservation::Monitoring { .. }));
        stop.send(true).expect("stop observer");
        task.await.expect("observer task").expect("observer error");
    }
}
