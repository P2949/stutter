#![allow(dead_code)]

use std::collections::BTreeMap;

use log::warn;
use tokio::sync::mpsc;

use crate::session_events::{MonitorEvent, MonitorEventDeliveryClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorEventReliability {
    Reliable,
    Droppable,
    Conflated,
}

#[derive(Debug)]
pub struct MonitorEventBus {
    tx: Option<mpsc::Sender<MonitorEvent>>,
    dropped: BTreeMap<&'static str, u64>,
    conflated_interval: Option<MonitorEvent>,
}

impl MonitorEventBus {
    pub fn new(tx: Option<mpsc::Sender<MonitorEvent>>) -> Self {
        Self {
            tx,
            dropped: BTreeMap::new(),
            conflated_interval: None,
        }
    }

    pub async fn emit(&mut self, event: MonitorEvent) {
        match reliability_for_event(&event) {
            MonitorEventReliability::Reliable => self.emit_reliable(event).await,
            MonitorEventReliability::Conflated => self.emit_conflated(event),
            MonitorEventReliability::Droppable => self.emit_droppable(event),
        }
    }

    pub async fn flush(&mut self) {
        if let Some(event) = self.conflated_interval.take() {
            self.emit_reliable(event).await;
        }
    }

    pub fn dropped_counts(&self) -> &BTreeMap<&'static str, u64> {
        &self.dropped
    }

    pub fn has_drops(&self) -> bool {
        self.dropped.values().any(|count| *count > 0)
    }

    async fn emit_reliable(&mut self, event: MonitorEvent) {
        let Some(tx) = &self.tx else {
            return;
        };

        if let Err(err) = tx.send(event).await {
            warn!("monitor_event_channel_closed err={err}");
        }
    }

    fn emit_conflated(&mut self, event: MonitorEvent) {
        let Some(tx) = &self.tx else {
            return;
        };

        match tx.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(event)) => {
                self.conflated_interval = Some(event);
                *self.dropped.entry("interval_conflated").or_default() += 1;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                *self.dropped.entry("channel_closed").or_default() += 1;
            }
        }
    }

    fn emit_droppable(&mut self, event: MonitorEvent) {
        let Some(tx) = &self.tx else {
            return;
        };

        let kind = event.kind();
        if let Err(err) = tx.try_send(event) {
            *self.dropped.entry(kind).or_default() += 1;
            warn!("monitor_event_dropped dropped_event={} err={}", kind, err);
        }
    }
}

pub fn reliability_for_event(event: &MonitorEvent) -> MonitorEventReliability {
    match event.delivery_class() {
        MonitorEventDeliveryClass::Reliable => MonitorEventReliability::Reliable,
        MonitorEventDeliveryClass::Conflated => MonitorEventReliability::Conflated,
        MonitorEventDeliveryClass::Droppable => MonitorEventReliability::Droppable,
    }
}
