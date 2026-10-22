//! GRUB through `grub-editenv`/`grub-reboot`. One-shot selection only (`grub-reboot`), which
//! needs `GRUB_DEFAULT=saved`; the saved default is never changed. Entries come from the
//! generated `grub.cfg` (read-only): top-level entries and submenu entries, by their menu id.
use std::sync::Arc;

use super::{BootAdapter, BootEntry, Shell, running_kernel_release};

pub struct Grub {
    shell: Arc<dyn Shell>,
    config: String,
}

impl Grub {
    pub fn new(shell: Arc<dyn Shell>, config: &str) -> Self {
        Self {
            shell,
            config: config.to_owned(),
        }
    }

    fn editenv(&self) -> Result<String, String> {
        self.shell.run("grub-editenv", &["-", "list"])
    }

    fn reboot_tool(&self) -> &'static str {
        if self.shell.run("grub-reboot", &["--version"]).is_ok() {
            "grub-reboot"
        } else {
            "grub2-reboot"
        }
    }
}

/// Text inside the first pair of single quotes after `marker` on `line`.
fn quoted_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let start = rest.find('\'')? + 1;
    let end = rest[start..].find('\'')?;
    Some(&rest[start..start + end])
}

/// Parses `menuentry` blocks, nesting `submenu`s into `>`-joined ids as `grub-reboot` expects.
pub fn parse_grub_cfg(text: &str) -> Vec<BootEntry> {
    let mut entries = Vec::new();
    let mut submenu_stack: Vec<(String, usize)> = Vec::new();
    let mut depth = 0_usize;
    let mut current: Option<BootEntry> = None;
    let mut current_depth = 0_usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("submenu ") {
            let id = quoted_after(trimmed, "$menuentry_id_option")
                .unwrap_or("submenu")
                .to_owned();
            submenu_stack.push((id, depth));
        }
        if trimmed.starts_with("menuentry ") {
            let id = quoted_after(trimmed, "$menuentry_id_option");
            let title = trimmed.split('\'').nth(1).unwrap_or("").to_owned();
            if let Some(id) = id {
                let full = submenu_stack
                    .iter()
                    .map(|(sub, _)| sub.as_str())
                    .chain(std::iter::once(id))
                    .collect::<Vec<_>>()
                    .join(">");
                current = Some(BootEntry {
                    id: full,
                    title,
                    kernel_image: None,
                    version: None,
                });
                current_depth = depth;
            }
        }
        if let Some(entry) = current.as_mut() {
            let words = trimmed.split_whitespace().collect::<Vec<_>>();
            if matches!(words.first(), Some(&"linux" | &"linux16" | &"linuxefi"))
                && let Some(image) = words.get(1)
            {
                entry.kernel_image = Some((*image).to_owned());
                entry.version = image.rsplit_once("vmlinuz-").map(|(_, v)| v.to_owned());
            }
        }
        depth += trimmed.matches('{').count();
        let closes = trimmed.matches('}').count();
        for _ in 0..closes {
            depth = depth.saturating_sub(1);
            if current.is_some() && depth == current_depth {
                entries.extend(current.take());
            }
            while submenu_stack.last().is_some_and(|(_, d)| *d == depth) {
                submenu_stack.pop();
            }
        }
    }
    // Only entries that boot a Linux image are kernel candidates.
    entries
        .into_iter()
        .filter(|entry| entry.kernel_image.is_some())
        .collect()
}

impl BootAdapter for Grub {
    fn backend(&self) -> &'static str {
        "grub"
    }

    fn discover_entries(&self) -> Result<Vec<BootEntry>, String> {
        Ok(parse_grub_cfg(&self.shell.read_file(&self.config)?))
    }

    fn current_entry(&self) -> Result<Option<String>, String> {
        // GRUB does not record which entry booted; match the running release against the
        // entries' kernel versions, and report nothing if that is ambiguous.
        let release = running_kernel_release()?;
        let matching = self
            .discover_entries()?
            .into_iter()
            .filter(|entry| entry.version.as_deref() == Some(release.as_str()))
            // Recovery-mode twins of the same kernel are not the normal entry.
            .filter(|entry| !entry.id.contains("recovery"))
            .collect::<Vec<_>>();
        Ok(match matching.as_slice() {
            [only] => Some(only.id.clone()),
            [first, ..] => Some(first.id.clone()),
            [] => None,
        })
    }

    fn default_entry(&self) -> Result<Option<String>, String> {
        Ok(self
            .editenv()?
            .lines()
            .find_map(|line| line.strip_prefix("saved_entry="))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()))
    }

    fn current_kernel_release(&self) -> Result<String, String> {
        running_kernel_release()
    }

    fn set_oneshot(&self, entry_id: &str) -> Result<(), String> {
        if self.default_entry()?.is_none() {
            return Err("GRUB_DEFAULT=saved is required for one-shot boot selection".to_owned());
        }
        self.shell.run(self.reboot_tool(), &[entry_id]).map(drop)
    }

    fn restore_default(&self) -> Result<(), String> {
        // `next_entry` is the one-shot variable; clearing it leaves the saved default alone.
        let _ = self
            .shell
            .run("grub-editenv", &["-", "unset", "next_entry"]);
        Ok(())
    }

    fn reboot(&self) -> Result<(), String> {
        self.shell.run("systemctl", &["reboot"]).map(drop)
    }
}
