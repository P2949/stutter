use super::SpikeEvent;

pub const MAX_SPIKE_EVENTS: usize = 500_000;

#[derive(Debug, PartialEq, Eq)]
pub enum SpikePushResult {
    Stored,
    Dropped,
}

#[derive(Debug)]
pub struct SpikeEventBuffer {
    pub(super) events: Vec<SpikeEvent>,
    pub(super) truncated: bool,
    max_events: u64,
}

impl SpikeEventBuffer {
    pub fn new(max_events: u64) -> Self {
        Self {
            events: Vec::with_capacity(1024.min(max_events as usize)),
            truncated: false,
            max_events,
        }
    }

    pub fn push(&mut self, event: SpikeEvent) -> SpikePushResult {
        if (self.events.len() as u64) < self.max_events {
            self.events.push(event);
            SpikePushResult::Stored
        } else {
            self.truncated = true;
            SpikePushResult::Dropped
        }
    }

    #[cfg(test)]
    pub fn truncate(&mut self) {
        self.truncated = true;
    }

    #[cfg(test)]
    pub fn with_max_events(max_events: u64) -> Self {
        Self {
            events: Vec::new(),
            truncated: false,
            max_events,
        }
    }

    #[cfg(test)]
    pub fn as_slice(&self) -> &[SpikeEvent] {
        &self.events
    }

    #[cfg(test)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl Default for SpikeEventBuffer {
    fn default() -> Self {
        Self::new(MAX_SPIKE_EVENTS as u64)
    }
}
