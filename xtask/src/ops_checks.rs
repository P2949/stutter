use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use crate::process::run_process;

pub fn run_service_smoke(root: &Path) -> anyhow::Result<()> {
    let systemd = root.join("packaging/systemd");
    let openrc = root.join("packaging/openrc");

    let low_risk = read(systemd.join("stutter-autotune-low-risk.service"))?;
    require_contains(
        &low_risk,
        "ExecStop=/usr/bin/stutter daemon emergency-restore",
    )?;
    require_contains(&low_risk, "Environment=HOME=/var/lib/stutter")?;
    require_contains(&low_risk, "STUTTER_AUTOTUNE_MODE=apply-low-risk")?;
    require_contains(
        &low_risk,
        "only supports STUTTER_AUTOTUNE_MODE=apply-low-risk",
    )?;

    let observe = read(systemd.join("stutter-autotune-observe.service"))?;
    require_contains(&observe, "Environment=HOME=/var/lib/stutter")?;
    require_contains(&observe, "STUTTER_AUTOTUNE_MODE=observe")?;
    require_contains(&observe, "only supports STUTTER_AUTOTUNE_MODE=observe")?;
    require_not_contains(&observe, "--mode apply-low-risk")?;

    let agent = read(systemd.join("stutter-agent.service"))?;
    require_contains(&agent, "ExecStart=/usr/bin/stutter agent --unix-socket")?;
    require_contains(&agent, "ExecStop=/usr/bin/stutter daemon emergency-restore")?;
    require_contains(&agent, "Environment=HOME=/var/lib/stutter")?;

    let openrc_low = read(openrc.join("stutter-autotune-low-risk"))?;
    require_contains(&openrc_low, "daemon emergency-restore")?;
    require_contains(&openrc_low, "stutter_mode:=apply-low-risk")?;
    require_contains(&openrc_low, "only supports stutter_mode=apply-low-risk")?;

    let openrc_observe = read(openrc.join("stutter-autotune-observe"))?;
    require_contains(&openrc_observe, "stutter_mode:=observe")?;
    require_contains(&openrc_observe, "only supports stutter_mode=observe")?;
    require_not_contains(&openrc_observe, "--mode apply-low-risk")?;

    let openrc_agent = read(openrc.join("stutter-agent"))?;
    require_contains(&openrc_agent, "agent --unix-socket")?;
    require_contains(&openrc_agent, "daemon emergency-restore")?;

    println!("service smoke ok");
    Ok(())
}

pub fn run_package_layout_check(root: &Path) -> anyhow::Result<()> {
    let manifest = read(root.join("packaging/tarball/MANIFEST.txt"))?;
    for entry in manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        match entry {
            "bin/stutter" | "lib/stutter/stutter.bpf.o" => {}
            "etc/stutter/" | "var/lib/stutter/" | "var/log/stutter/" => {}
            doc if doc.starts_with("share/doc/stutter/") => {
                let filename = doc.trim_start_matches("share/doc/stutter/");
                require_path(root.join("docs").join(filename))?;
            }
            systemd if systemd.starts_with("share/stutter/systemd/") => {
                let filename = systemd.trim_start_matches("share/stutter/systemd/");
                require_path(root.join("packaging/systemd").join(filename))?;
            }
            openrc if openrc.starts_with("share/stutter/openrc/") => {
                let filename = openrc.trim_start_matches("share/stutter/openrc/");
                require_path(root.join("packaging/openrc").join(filename))?;
            }
            other => bail!("unhandled tarball manifest entry {other:?}"),
        }
    }

    let pkgbuild = read(root.join("packaging/arch/PKGBUILD"))?;
    require_contains(&pkgbuild, "docs/INSTALL.md")?;
    require_contains(&pkgbuild, "docs/PACKAGING.md")?;

    let ebuild = read(root.join("packaging/gentoo/stutter-9999.ebuild"))?;
    require_contains(&ebuild, "intentionally not production-ready")?;

    println!("package layout check ok");
    Ok(())
}

pub fn run_local_install_smoke(root: &Path) -> anyhow::Result<()> {
    run_process(root, "bash", &["scripts/smoke-local-install.sh"])
}

fn read(path: PathBuf) -> anyhow::Result<String> {
    fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
}

fn require_path(path: PathBuf) -> anyhow::Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("missing required packaging path {}", path.display())
    }
}

fn require_contains(haystack: &str, needle: &str) -> anyhow::Result<()> {
    if haystack.contains(needle) {
        Ok(())
    } else {
        bail!("expected service/package text to contain {needle:?}")
    }
}

fn require_not_contains(haystack: &str, needle: &str) -> anyhow::Result<()> {
    if haystack.contains(needle) {
        bail!("service/package text must not contain {needle:?}")
    } else {
        Ok(())
    }
}
