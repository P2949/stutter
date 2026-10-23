//! Boot control. Policy (authorized entries, reboot budget, durable state before a reboot) lives
//! in the server; this module only performs the mechanical steps through a bootloader adapter.
//! Adapters read the bootloader and set a *one-shot* next entry; they never rewrite the user's
//! boot configuration, change the default entry, download kernels or kexec anything.
pub mod grub;
pub mod systemd_boot;

use std::{process::Command, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One bootable entry as the bootloader reports it. The kernel is never inferred from the title.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootEntry {
    pub id: String,
    pub title: String,
    /// Kernel image path or identity when the bootloader exposes it.
    pub kernel_image: Option<String>,
    pub version: Option<String>,
}

/// Runs a program and returns its stdout; `Err` on launch failure or non-zero exit. A trait so
/// adapters are tested against canned output instead of a real bootloader.
pub trait Shell: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
    fn read_file(&self, path: &str) -> Result<String, String>;
    fn exists(&self, path: &str) -> bool;
}

pub struct SystemShell;

impl Shell for SystemShell {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|error| format!("could not run {program}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "{program} {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|error| format!("{program} output: {error}"))
    }
    fn read_file(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|error| format!("read {path}: {error}"))
    }
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
}

pub trait BootAdapter: Send + Sync {
    fn backend(&self) -> &'static str;
    fn discover_entries(&self) -> Result<Vec<BootEntry>, String>;
    /// The entry this boot came from, when the bootloader can tell.
    fn current_entry(&self) -> Result<Option<String>, String>;
    fn default_entry(&self) -> Result<Option<String>, String>;
    /// `uname -r` of the running kernel: the only authority on which kernel is observed.
    fn current_kernel_release(&self) -> Result<String, String>;
    /// Selects `entry_id` for the next boot only.
    fn set_oneshot(&self, entry_id: &str) -> Result<(), String>;
    /// Drops any pending one-shot selection; never touches the default entry.
    fn restore_default(&self) -> Result<(), String>;
    fn reboot(&self) -> Result<(), String>;
}

/// `uname -r` without a shell.
pub fn running_kernel_release() -> Result<String, String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|release| release.trim().to_owned())
        .map_err(|error| format!("read the kernel release: {error}"))
}

/// Entries with distinct kernel identity, keeping the first of each. The kernel image the entry
/// boots is the identity: two entries naming the same image file boot the same kernel whatever
/// their titles or generation numbers say (NixOS keeps dozens of generations on one image). When
/// the bootloader exposes no image the version string is used; entries with neither stay
/// distinct by id because they cannot be proven duplicates.
pub fn distinct_candidates(entries: &[BootEntry]) -> Vec<BootEntry> {
    let mut seen = std::collections::BTreeSet::new();
    let mut candidates = Vec::new();
    for entry in entries {
        let identity = match (&entry.kernel_image, &entry.version) {
            (Some(image), _) => format!("image:{image}"),
            (None, Some(version)) => format!("version:{version}"),
            (None, None) => format!("id:{}", entry.id),
        };
        if seen.insert(identity) {
            candidates.push(entry.clone());
        }
    }
    candidates
}

/// Picks the adapter for this host, or `None` when the bootloader is unknown (the kernel stage
/// then stops instead of guessing).
pub fn detect_adapter(shell: Arc<dyn Shell>) -> Option<Box<dyn BootAdapter>> {
    if shell.exists("/sys/firmware/efi") && shell.run("bootctl", &["is-installed"]).is_ok() {
        return Some(Box::new(systemd_boot::SystemdBoot::new(shell)));
    }
    ["/boot/grub/grub.cfg", "/boot/grub2/grub.cfg"]
        .into_iter()
        .find(|path| shell.exists(path))
        .map(|path| Box::new(grub::Grub::new(shell, path)) as Box<dyn BootAdapter>)
}

/// What the server calls. A trait of its own so tests can inject a fake without a bootloader.
pub trait BootControl: Send + Sync {
    fn set_next_boot(&self, entry_id: &str) -> Result<(), String>;
    fn reboot(&self) -> Result<(), String>;
    /// Drops any pending one-shot selection (cancellation); never touches the default entry.
    fn restore_default(&self) -> Result<(), String> {
        Ok(())
    }
    /// Boot facts included in `Status`: backend, current/default entry, running kernel, entries.
    fn describe(&self) -> Value {
        json!({"backend": "unsupported"})
    }
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

pub struct AdapterControl(pub Box<dyn BootAdapter>);

impl BootControl for AdapterControl {
    fn set_next_boot(&self, entry_id: &str) -> Result<(), String> {
        self.0.set_oneshot(entry_id)
    }
    fn reboot(&self) -> Result<(), String> {
        self.0.reboot()
    }
    fn restore_default(&self) -> Result<(), String> {
        self.0.restore_default()
    }
    fn describe(&self) -> Value {
        let adapter = &self.0;
        json!({
            "backend": adapter.backend(),
            "current_entry": adapter.current_entry().ok().flatten(),
            "default_entry": adapter.default_entry().ok().flatten(),
            "kernel_release": adapter.current_kernel_release().ok(),
            "entries": adapter.discover_entries().unwrap_or_default(),
        })
    }
}
