use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::authorization;
use crate::{
    boot::{
        AdapterControl, BootAdapter, BootControl, BootEntry, Shell, detect_adapter,
        distinct_candidates,
        grub::{Grub, parse_grub_cfg},
        systemd_boot::SystemdBoot,
    },
    install::apply_boot_discovery,
};

/// Canned shell: maps "program arg arg" to stdout (or an error) and records every call.
#[derive(Default)]
struct FakeShell {
    outputs: BTreeMap<String, Result<String, String>>,
    files: BTreeMap<String, String>,
    existing: Vec<String>,
    calls: Mutex<Vec<String>>,
}

impl FakeShell {
    fn with(mut self, command: &str, output: Result<&str, &str>) -> Self {
        self.outputs.insert(
            command.to_owned(),
            output.map(str::to_owned).map_err(str::to_owned),
        );
        self
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Shell for FakeShell {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let command = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.lock().unwrap().push(command.clone());
        self.outputs
            .get(&command)
            .cloned()
            .unwrap_or_else(|| Err(format!("unexpected command {command}")))
    }
    fn read_file(&self, path: &str) -> Result<String, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("no file {path}"))
    }
    fn exists(&self, path: &str) -> bool {
        self.existing.iter().any(|p| p == path)
    }
}

const BOOTCTL: &str = r#"[
 {"id":"arch.conf","title":"Arch Linux","showTitle":"Arch Linux","version":"6.18.55-1","linux":"/vmlinuz-linux","isDefault":true,"isSelected":true},
 {"id":"arch-lts.conf","title":"Arch Linux LTS","version":"6.12.4-lts","linux":"/vmlinuz-linux-lts","isDefault":false,"isSelected":false},
 {"id":"arch-fallback.conf","title":"Arch Linux (fallback)","version":"6.18.55-1","linux":"/vmlinuz-linux","isDefault":false,"isSelected":false},
 {"id":"auto-reboot-to-firmware-setup","title":"Reboot Into Firmware Interface","isDefault":false,"isSelected":false}
]"#;

#[test]
fn systemd_boot_reads_entries_selects_one_shot_and_never_changes_the_default() {
    let shell = Arc::new(
        FakeShell::default()
            .with("bootctl list --json=short", Ok(BOOTCTL))
            .with("bootctl set-oneshot arch-lts.conf", Ok(""))
            .with("bootctl set-oneshot ", Ok(""))
            .with("systemctl reboot", Ok("")),
    );
    let adapter = SystemdBoot::new(shell.clone());
    let entries = adapter.discover_entries().unwrap();
    assert_eq!(
        entries.len(),
        3,
        "the firmware entry is not a kernel candidate"
    );
    assert_eq!(
        adapter.default_entry().unwrap().as_deref(),
        Some("arch.conf")
    );
    assert_eq!(
        adapter.current_entry().unwrap().as_deref(),
        Some("arch.conf")
    );
    adapter.set_oneshot("arch-lts.conf").unwrap();
    adapter.restore_default().unwrap();
    adapter.reboot().unwrap();
    let calls = shell.calls();
    assert!(calls.contains(&"bootctl set-oneshot arch-lts.conf".to_owned()));
    assert!(
        !calls.iter().any(|c| c.contains("set-default")),
        "{calls:?}"
    );
    assert_eq!(adapter.backend(), "systemd-boot");
    // A failing bootctl is an error, never a silent success.
    let failing = SystemdBoot::new(Arc::new(
        FakeShell::default().with("bootctl set-oneshot x", Err("denied")),
    ));
    assert!(failing.set_oneshot("x").unwrap_err().contains("denied"));
    let garbage = SystemdBoot::new(Arc::new(
        FakeShell::default().with("bootctl list --json=short", Ok("not json")),
    ));
    assert!(garbage.discover_entries().is_err());
}

const GRUB_CFG: &str = r#"
menuentry 'Gentoo GNU/Linux' --class gentoo --class gnu-linux $menuentry_id_option 'gnulinux-simple-aaaa' {
    linux /boot/vmlinuz-6.18.55-gentoo root=/dev/sda2 ro
    initrd /boot/initramfs-6.18.55-gentoo.img
}
submenu 'Advanced options for Gentoo GNU/Linux' $menuentry_id_option 'gnulinux-advanced-aaaa' {
    menuentry 'Gentoo GNU/Linux, with Linux 6.12.4-gentoo' --class gentoo $menuentry_id_option 'gnulinux-6.12.4-gentoo-advanced-aaaa' {
        linux /boot/vmlinuz-6.12.4-gentoo root=/dev/sda2 ro
    }
    menuentry 'Gentoo GNU/Linux, with Linux 6.12.4-gentoo (recovery mode)' --class gentoo $menuentry_id_option 'gnulinux-6.12.4-gentoo-recovery-aaaa' {
        linux /boot/vmlinuz-6.12.4-gentoo root=/dev/sda2 ro single
    }
}
menuentry 'UEFI Firmware Settings' $menuentry_id_option 'uefi-firmware' {
    fwsetup
}
"#;

#[test]
fn grub_cfg_parsing_nests_submenu_ids_and_skips_non_linux_entries() {
    let entries = parse_grub_cfg(GRUB_CFG);
    let ids = entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "gnulinux-simple-aaaa",
            "gnulinux-advanced-aaaa>gnulinux-6.12.4-gentoo-advanced-aaaa",
            "gnulinux-advanced-aaaa>gnulinux-6.12.4-gentoo-recovery-aaaa",
        ]
    );
    assert_eq!(entries[0].version.as_deref(), Some("6.18.55-gentoo"));
    assert_eq!(
        entries[1].kernel_image.as_deref(),
        Some("/boot/vmlinuz-6.12.4-gentoo")
    );
}

#[test]
fn grub_selects_one_shot_only_with_a_saved_default_and_never_changes_it() {
    let mut shell = FakeShell::default()
        .with(
            "grub-editenv - list",
            Ok("saved_entry=gnulinux-simple-aaaa\n"),
        )
        .with("grub-reboot --version", Ok("grub-reboot 2.12"))
        .with(
            "grub-reboot gnulinux-advanced-aaaa>gnulinux-6.12.4-gentoo-advanced-aaaa",
            Ok(""),
        )
        .with("grub-editenv - unset next_entry", Ok(""))
        .with("systemctl reboot", Ok(""));
    shell
        .files
        .insert("/boot/grub/grub.cfg".to_owned(), GRUB_CFG.to_owned());
    let shell = Arc::new(shell);
    let adapter = Grub::new(shell.clone(), "/boot/grub/grub.cfg");
    assert_eq!(
        adapter.default_entry().unwrap().as_deref(),
        Some("gnulinux-simple-aaaa")
    );
    assert_eq!(adapter.discover_entries().unwrap().len(), 3);
    adapter
        .set_oneshot("gnulinux-advanced-aaaa>gnulinux-6.12.4-gentoo-advanced-aaaa")
        .unwrap();
    adapter.restore_default().unwrap();
    adapter.reboot().unwrap();
    assert!(
        !shell
            .calls()
            .iter()
            .any(|c| c.contains("set_default") || c.contains("saved_entry=") && c.contains("set"))
    );
    // Without GRUB_DEFAULT=saved a one-shot selection is refused instead of attempted.
    let unsaved = Grub::new(
        Arc::new(
            FakeShell::default().with("grub-editenv - list", Ok("# GRUB Environment Block\n")),
        ),
        "/boot/grub/grub.cfg",
    );
    assert!(
        unsaved
            .set_oneshot("x")
            .unwrap_err()
            .contains("GRUB_DEFAULT=saved")
    );
}

#[test]
fn the_backend_is_detected_from_the_host_and_unknown_hosts_get_none() {
    let efi = FakeShell {
        existing: vec!["/sys/firmware/efi".to_owned()],
        ..FakeShell::default()
    }
    .with("bootctl is-installed", Ok(""));
    assert_eq!(
        detect_adapter(Arc::new(efi)).unwrap().backend(),
        "systemd-boot"
    );
    let grub = FakeShell {
        existing: vec!["/boot/grub/grub.cfg".to_owned()],
        ..FakeShell::default()
    };
    assert_eq!(detect_adapter(Arc::new(grub)).unwrap().backend(), "grub");
    assert!(
        detect_adapter(Arc::new(FakeShell::default())).is_none(),
        "an unknown bootloader is never guessed"
    );
}

fn entry(id: &str, image: Option<&str>, version: Option<&str>) -> BootEntry {
    BootEntry {
        id: id.to_owned(),
        title: id.to_owned(),
        kernel_image: image.map(str::to_owned),
        version: version.map(str::to_owned),
    }
}

#[test]
fn duplicate_kernels_collapse_to_one_candidate_but_unidentified_entries_stay_distinct() {
    let entries = vec![
        entry("a", Some("/vmlinuz"), Some("6.1")),
        entry("a-fallback", Some("/vmlinuz"), Some("6.1")),
        entry("b", Some("/vmlinuz-lts"), Some("6.12")),
        entry("x", None, None),
        entry("y", None, None),
    ];
    let ids = distinct_candidates(&entries)
        .into_iter()
        .map(|e| e.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["a", "b", "x", "y"]);
}

struct StaticAdapter {
    entries: Vec<BootEntry>,
    default: Option<String>,
}

impl BootAdapter for StaticAdapter {
    fn backend(&self) -> &'static str {
        "static"
    }
    fn discover_entries(&self) -> Result<Vec<BootEntry>, String> {
        Ok(self.entries.clone())
    }
    fn current_entry(&self) -> Result<Option<String>, String> {
        Ok(self.default.clone())
    }
    fn default_entry(&self) -> Result<Option<String>, String> {
        Ok(self.default.clone())
    }
    fn current_kernel_release(&self) -> Result<String, String> {
        Ok("6.1".to_owned())
    }
    fn set_oneshot(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn restore_default(&self) -> Result<(), String> {
        Ok(())
    }
    fn reboot(&self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn install_time_discovery_overwrites_staged_boot_fields_and_sets_the_budget() {
    let mut auth = authorization("/repo");
    auth.allowed_boot_entries = vec!["attacker-chosen".to_owned()];
    auth.max_reboots = 8;
    auth.reboot_slack = 2;
    let adapter = StaticAdapter {
        entries: vec![
            entry("a", Some("/vmlinuz"), Some("6.1")),
            entry("a-fallback", Some("/vmlinuz"), Some("6.1")),
            entry("b", Some("/vmlinuz-lts"), Some("6.12")),
            entry("c", Some("/vmlinuz-zen"), Some("6.17")),
        ],
        default: Some("a-fallback".to_owned()),
    };
    apply_boot_discovery(&mut auth, &adapter).unwrap();
    assert!(
        !auth
            .allowed_boot_entries
            .contains(&"attacker-chosen".to_owned())
    );
    assert_eq!(
        auth.allowed_boot_entries,
        ["a", "b", "c", "a-fallback"],
        "original is always returnable"
    );
    assert_eq!(auth.original_boot_entry.as_deref(), Some("a-fallback"));
    assert_eq!(auth.max_reboots, 5, "min(3 candidates + 2 slack, 8)");
    auth.max_reboots = 4;
    apply_boot_discovery(&mut auth, &adapter).unwrap();
    assert_eq!(auth.max_reboots, 4, "the manifest cap bounds the budget");
}

#[test]
fn a_hostile_bootloader_entry_id_never_becomes_authorized() {
    let mut auth = authorization("/repo");
    let adapter = StaticAdapter {
        entries: vec![
            entry("--help", Some("/vmlinuz"), Some("1")),
            entry("ok", Some("/v2"), Some("2")),
        ],
        default: Some("ok".to_owned()),
    };
    assert!(apply_boot_discovery(&mut auth, &adapter).is_err());
    for bad in ["a;b", "$(id)", "a b", "../x", "-x", ""] {
        let mut candidate = authorization("/repo");
        candidate.allowed_boot_entries = vec![bad.to_owned()];
        assert!(candidate.validate_shape().is_err(), "{bad:?}");
    }
    let mut nested = authorization("/repo");
    nested.allowed_boot_entries = vec!["gnulinux-advanced-aa>gnulinux-6.1-advanced-aa".to_owned()];
    assert!(
        nested.validate_shape().is_ok(),
        "GRUB nested ids are legitimate"
    );
}

#[test]
fn status_reports_the_boot_facts_the_kernel_stage_needs() {
    let control = AdapterControl(Box::new(StaticAdapter {
        entries: vec![entry("a", Some("/vmlinuz"), Some("6.1"))],
        default: Some("a".to_owned()),
    }));
    let described = control.describe();
    assert_eq!(described["backend"], "static");
    assert_eq!(described["default_entry"], "a");
    assert_eq!(described["kernel_release"], "6.1");
    assert_eq!(described["entries"][0]["id"], "a");
}
