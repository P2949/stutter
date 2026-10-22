//! systemd-boot through `bootctl`. One-shot selection only (`set-oneshot`); the default entry
//! is never changed.
use std::sync::Arc;

use serde_json::Value;

use super::{BootAdapter, BootEntry, Shell, running_kernel_release};

pub struct SystemdBoot {
    shell: Arc<dyn Shell>,
}

impl SystemdBoot {
    pub fn new(shell: Arc<dyn Shell>) -> Self {
        Self { shell }
    }

    fn list(&self) -> Result<Vec<Value>, String> {
        let output = self.shell.run("bootctl", &["list", "--json=short"])?;
        let parsed: Value = serde_json::from_str(&output)
            .map_err(|error| format!("bootctl list --json output is not JSON: {error}"))?;
        parsed
            .as_array()
            .cloned()
            .ok_or_else(|| "bootctl list --json did not return an array".to_owned())
    }
}

impl BootAdapter for SystemdBoot {
    fn backend(&self) -> &'static str {
        "systemd-boot"
    }

    fn discover_entries(&self) -> Result<Vec<BootEntry>, String> {
        Ok(self
            .list()?
            .iter()
            .filter_map(|entry| {
                let id = entry["id"].as_str()?.to_owned();
                Some(BootEntry {
                    title: entry["showTitle"]
                        .as_str()
                        .or_else(|| entry["title"].as_str())
                        .unwrap_or(&id)
                        .to_owned(),
                    kernel_image: entry["linux"].as_str().map(str::to_owned),
                    version: entry["version"].as_str().map(str::to_owned),
                    id,
                })
            })
            // Only entries that boot a Linux image are kernel candidates (not firmware/UKI-less).
            .filter(|entry| entry.kernel_image.is_some())
            .collect())
    }

    fn current_entry(&self) -> Result<Option<String>, String> {
        Ok(self
            .list()?
            .iter()
            .find(|entry| entry["isSelected"] == true)
            .and_then(|entry| entry["id"].as_str().map(str::to_owned)))
    }

    fn default_entry(&self) -> Result<Option<String>, String> {
        Ok(self
            .list()?
            .iter()
            .find(|entry| entry["isDefault"] == true)
            .and_then(|entry| entry["id"].as_str().map(str::to_owned)))
    }

    fn current_kernel_release(&self) -> Result<String, String> {
        running_kernel_release()
    }

    fn set_oneshot(&self, entry_id: &str) -> Result<(), String> {
        self.shell
            .run("bootctl", &["set-oneshot", entry_id])
            .map(drop)
    }

    fn restore_default(&self) -> Result<(), String> {
        // Clearing the variable is best effort: the default entry was never changed.
        let _ = self.shell.run("bootctl", &["set-oneshot", ""]);
        Ok(())
    }

    fn reboot(&self) -> Result<(), String> {
        self.shell.run("systemctl", &["reboot"]).map(drop)
    }
}
