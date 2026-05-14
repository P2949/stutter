use std::collections::BTreeMap;

use log::warn;
use tokio::sync::mpsc;

use crate::session_events::{MonitorEvent, MonitorEventDeliveryClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorEventReliability {
    Reliable,
    Droppable,
    Conflated,
    AuditCritical,
}

#[derive(Debug)]
pub struct MonitorEventSubscriber {
    name: &'static str,
    tx: mpsc::Sender<MonitorEvent>,
}

impl MonitorEventSubscriber {
    pub fn new(name: &'static str, tx: mpsc::Sender<MonitorEvent>) -> Self {
        Self { name, tx }
    }
}

#[derive(Debug)]
pub struct MonitorEventBus {
    subscribers: Vec<MonitorEventSubscriber>,
    dropped: BTreeMap<String, u64>,
    conflated: BTreeMap<&'static str, MonitorEvent>,
}

impl MonitorEventBus {
    pub fn new(tx: Option<mpsc::Sender<MonitorEvent>>) -> Self {
        let subscribers = tx
            .map(|tx| vec![MonitorEventSubscriber::new("remote_stream", tx)])
            .unwrap_or_default();
        Self::with_subscribers(subscribers)
    }

    pub fn with_subscribers(subscribers: Vec<MonitorEventSubscriber>) -> Self {
        Self {
            subscribers,
            dropped: BTreeMap::new(),
            conflated: BTreeMap::new(),
        }
    }

    pub async fn emit(&mut self, event: MonitorEvent) {
        match reliability_for_event(&event) {
            MonitorEventReliability::Reliable => self.emit_reliable(event).await,
            MonitorEventReliability::AuditCritical => self.emit_audit_critical(event).await,
            MonitorEventReliability::Conflated => self.emit_conflated(event),
            MonitorEventReliability::Droppable => self.emit_droppable(event),
        }
    }

    pub async fn flush(&mut self) {
        let events = std::mem::take(&mut self.conflated)
            .into_values()
            .collect::<Vec<_>>();
        for event in events {
            self.emit_reliable(event).await;
        }
    }

    pub fn dropped_counts(&self) -> &BTreeMap<String, u64> {
        &self.dropped
    }

    pub fn has_drops(&self) -> bool {
        self.dropped.values().any(|count| *count > 0)
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    async fn emit_reliable(&mut self, event: MonitorEvent) {
        let subscribers = self.subscriber_handles();
        for (name, tx) in subscribers {
            if let Err(err) = tx.send(event.clone()).await {
                self.count_drop(name, event.kind());
                warn!(
                    "monitor_event_channel_closed subscriber={} event={} err={err}",
                    name,
                    event.kind(),
                );
            }
        }
    }

    async fn emit_audit_critical(&mut self, event: MonitorEvent) {
        let subscribers = self.subscriber_handles();
        for (name, tx) in subscribers {
            if let Err(err) = tx.send(event.clone()).await {
                self.count_drop(name, event.kind());
                warn!(
                    "audit_critical_monitor_event_undeliverable subscriber={} event={} err={err}",
                    name,
                    event.kind(),
                );
            }
        }
    }

    fn emit_conflated(&mut self, event: MonitorEvent) {
        let mut had_full_subscriber = false;
        let kind = event.kind();

        let subscribers = self.subscriber_handles();
        for (name, tx) in subscribers {
            match tx.try_send(event.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    had_full_subscriber = true;
                    self.count_drop(name, kind);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.count_drop(name, kind);
                }
            }
        }

        if had_full_subscriber {
            self.conflated.insert(kind, event);
        }
    }

    fn emit_droppable(&mut self, event: MonitorEvent) {
        let kind = event.kind();
        let subscribers = self.subscriber_handles();
        for (name, tx) in subscribers {
            if let Err(err) = tx.try_send(event.clone()) {
                self.count_drop(name, kind);
                warn!(
                    "monitor_event_dropped subscriber={} dropped_event={} err={}",
                    name, kind, err
                );
            }
        }
    }

    fn subscriber_handles(&self) -> Vec<(&'static str, mpsc::Sender<MonitorEvent>)> {
        self.subscribers
            .iter()
            .map(|subscriber| (subscriber.name, subscriber.tx.clone()))
            .collect()
    }

    fn count_drop(&mut self, subscriber: &'static str, kind: &'static str) {
        *self
            .dropped
            .entry(format!("{subscriber}:{kind}"))
            .or_default() += 1;
    }
}

pub fn reliability_for_event(event: &MonitorEvent) -> MonitorEventReliability {
    match event.delivery_class() {
        MonitorEventDeliveryClass::Reliable => MonitorEventReliability::Reliable,
        MonitorEventDeliveryClass::Conflated => MonitorEventReliability::Conflated,
        MonitorEventDeliveryClass::Droppable => MonitorEventReliability::Droppable,
        MonitorEventDeliveryClass::AuditCritical => MonitorEventReliability::AuditCritical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished_event(reason: &str) -> MonitorEvent {
        MonitorEvent::Finished {
            reason: reason.to_owned(),
        }
    }

    fn interval_event(elapsed_ms: u64) -> MonitorEvent {
        MonitorEvent::Interval {
            elapsed_ms,
            records: Vec::new(),
            drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
        }
    }

    #[tokio::test]
    async fn reliable_events_fan_out_to_all_subscribers() {
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        let mut bus = MonitorEventBus::with_subscribers(vec![
            MonitorEventSubscriber::new("recorder", a_tx),
            MonitorEventSubscriber::new("autotune", b_tx),
        ]);

        bus.emit(finished_event("done")).await;

        assert_eq!(a_rx.recv().await.unwrap().kind(), "finished");
        assert_eq!(b_rx.recv().await.unwrap().kind(), "finished");
        assert!(!bus.has_drops());
    }

    #[tokio::test]
    async fn droppable_events_are_dropped_and_counted_under_load() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut bus = MonitorEventBus::with_subscribers(vec![MonitorEventSubscriber::new(
            "remote_stream",
            tx,
        )]);

        bus.emit(MonitorEvent::LiveDiagnosis {
            entry: Box::new(crate::diagnosis::LiveDiagnosisEntry {
                elapsed_ms: 1,
                cause: crate::diagnosis::StutterCause::Unknown,
                confidence: crate::diagnosis::Confidence::Low,
                anchor_class: crate::process_tree::TaskClass::Unknown,
                anchor_comm: "test".to_owned(),
                evidence: Vec::new(),
            }),
        })
        .await;
        bus.emit(MonitorEvent::LiveDiagnosis {
            entry: Box::new(crate::diagnosis::LiveDiagnosisEntry {
                elapsed_ms: 2,
                cause: crate::diagnosis::StutterCause::Unknown,
                confidence: crate::diagnosis::Confidence::Low,
                anchor_class: crate::process_tree::TaskClass::Unknown,
                anchor_comm: "test".to_owned(),
                evidence: Vec::new(),
            }),
        })
        .await;

        assert_eq!(rx.recv().await.unwrap().elapsed_ms(), Some(1));
        assert_eq!(
            bus.dropped_counts().get("remote_stream:live_diagnosis"),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn conflated_events_flush_latest_sample() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut bus = MonitorEventBus::with_subscribers(vec![MonitorEventSubscriber::new(
            "status_cache",
            tx,
        )]);

        bus.emit(interval_event(1)).await;
        bus.emit(interval_event(2)).await;
        bus.emit(interval_event(3)).await;
        assert_eq!(rx.recv().await.unwrap().elapsed_ms(), Some(1));

        bus.flush().await;

        assert_eq!(rx.recv().await.unwrap().elapsed_ms(), Some(3));
        assert_eq!(bus.dropped_counts().get("status_cache:interval"), Some(&2));
    }
}
