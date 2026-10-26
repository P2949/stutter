mod acceptance;
mod action;
pub mod handler;
pub mod models;
mod system;
mod validate;

#[cfg(test)]
mod tests;

pub use acceptance::verify_uclamp_lifecycle;
pub(crate) use handler::UclampRollbackHandler;
pub use models::{UclampAction, UclampValues};

/// The `uclamp.min` / `uclamp.max` a task currently has (acceptance harnesses read it back).
pub(crate) fn read_current_uclamp(tid: u32) -> anyhow::Result<(u32, u32)> {
    let current = system::read_task_uclamp_from_sched_at(std::path::Path::new("/proc"), tid)?;
    Ok((current.sched_util_min, current.sched_util_max))
}
