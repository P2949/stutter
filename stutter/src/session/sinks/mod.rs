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

#[cfg(test)]
pub(crate) use alert::AlertSink;
pub use model::MonitorOutputConfig;
#[cfg(test)]
pub use model::{MonitorEventSink, MonitorSinkContext};
#[cfg(test)]
pub(crate) use recorder::RecorderSink;
pub use registry::{MonitorOutputSinkRegistry, MonitorOutputSinks};
