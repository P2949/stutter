mod alert;
mod error;
mod model;
mod otel;
mod prometheus;
mod recorder;
mod registry;
mod stdout;

#[cfg(test)]
mod tests;

pub(crate) use alert::AlertSink;
pub use error::SinkError;
pub use model::{MonitorEventSink, MonitorOutputConfig, MonitorSinkContext};
pub(crate) use otel::OtelSink;
pub(crate) use prometheus::PrometheusSink;
pub(crate) use recorder::RecorderSink;
pub use registry::{MonitorOutputSinkRegistry, MonitorOutputSinks};
pub(crate) use stdout::StdoutSink;
