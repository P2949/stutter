mod acceptance;
pub mod model;
mod prepared;
#[cfg(test)]
mod tests;

pub use acceptance::verify_nice_lifecycle;
pub use model::*;

/// The niceness a task currently has (acceptance harnesses read it back).
pub(crate) fn read_current_nice(tid: u32) -> anyhow::Result<i32> {
    use anyhow::Context;
    let stat = std::fs::read_to_string(format!("/proc/{tid}/stat"))?;
    stat.rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(16))
        .and_then(|field| field.parse().ok())
        .context("nice field missing from stat")
}
