#![allow(dead_code)] // Transitional sysfs façade.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

use std::path::Path;

pub(crate) fn read_to_string(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
