use std::{env, path::PathBuf};

use anyhow::{Context as _, anyhow};
use aya_build::Toolchain;

fn main() -> anyhow::Result<()> {
    ensure_cargo_bin_on_path();

    let cargo_metadata::Metadata { packages, .. } = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("MetadataCommand::exec")?;
    let ebpf_package = packages
        .into_iter()
        .find(|cargo_metadata::Package { name, .. }| name.as_str() == "stutter-ebpf")
        .ok_or_else(|| anyhow!("stutter-ebpf package not found"))?;
    let cargo_metadata::Package {
        name,
        manifest_path,
        ..
    } = ebpf_package;
    let ebpf_package = aya_build::Package {
        name: name.as_str(),
        root_dir: manifest_path
            .parent()
            .ok_or_else(|| anyhow!("no parent for {manifest_path}"))?
            .as_str(),
        ..Default::default()
    };
    aya_build::build_ebpf([ebpf_package], Toolchain::default())
}

fn ensure_cargo_bin_on_path() {
    let Some(home) = env::var_os("HOME") else {
        return;
    };

    let cargo_bin = PathBuf::from(home).join(".cargo").join("bin");
    if !cargo_bin.join("rustup").exists() {
        return;
    }

    let current_path = env::var_os("PATH").unwrap_or_default();
    if env::split_paths(&current_path).any(|path| path == cargo_bin) {
        return;
    }

    let mut paths = vec![cargo_bin];
    paths.extend(env::split_paths(&current_path));

    if let Ok(path) = env::join_paths(paths) {
        // SAFETY: build scripts are single-threaded here, and this happens before
        // aya-build spawns the cargo/rustup child processes that need the PATH.
        unsafe {
            env::set_var("PATH", path);
        }
    }
}
