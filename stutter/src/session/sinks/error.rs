use std::fmt;

#[derive(Debug)]
pub struct SinkError {
    pub sink: &'static str,
    pub event_kind: &'static str,
    pub message: String,
}

impl SinkError {
    pub fn new(
        sink: &'static str,
        event_kind: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            sink,
            event_kind,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "monitor event sink {} failed for {}: {}",
            self.sink, self.event_kind, self.message
        )
    }
}

impl std::error::Error for SinkError {}
