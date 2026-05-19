#![allow(dead_code)] // Transitional home for focus test builders.

#[cfg(test)]
pub(crate) fn test_process_name(pid: u32) -> String {
    format!("process-{pid}")
}
