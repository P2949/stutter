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

#[allow(unused_imports)]
pub(crate) use alert::AlertSink;
#[allow(unused_imports)]
pub use error::SinkError;
#[allow(unused_imports)]
pub use model::{MonitorEventSink, MonitorOutputConfig, MonitorSinkContext};
#[allow(unused_imports)]
pub(crate) use otel::OtelSink;
#[allow(unused_imports)]
pub(crate) use prometheus::PrometheusSink;
#[allow(unused_imports)]
pub(crate) use recorder::RecorderSink;
#[allow(unused_imports)]
pub use registry::{MonitorOutputSinkRegistry, MonitorOutputSinks};
#[allow(unused_imports)]
pub(crate) use stdout::StdoutSink;
