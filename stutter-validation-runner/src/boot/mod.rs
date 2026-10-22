//! Boot control seam. The server owns the policy (authorized entries, reboot budget, durable
//! state written before any reboot); a `BootControl` only performs the mechanical step.
//! Real adapters (systemd-boot, GRUB) arrive with the multi-kernel phase.

pub trait BootControl: Send + Sync {
    /// Selects `entry_id` for the next boot only.
    fn set_next_boot(&self, entry_id: &str) -> Result<(), String>;
    fn reboot(&self) -> Result<(), String>;
}

/// Used until a boot adapter is configured: every boot action fails closed.
pub struct UnsupportedBoot;

impl BootControl for UnsupportedBoot {
    fn set_next_boot(&self, _entry_id: &str) -> Result<(), String> {
        Err("no boot adapter is configured on this runner".to_owned())
    }
    fn reboot(&self) -> Result<(), String> {
        Err("no boot adapter is configured on this runner".to_owned())
    }
}
