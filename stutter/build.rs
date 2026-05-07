use std::{env, path::PathBuf};

use anyhow::{Context as _, anyhow};
use aya_build::Toolchain;

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-env-changed=STUTTER_USE_PREBUILT_BPF");
    println!("cargo:rerun-if-env-changed=STUTTER_BPF_OBJECT");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let out_object = out_dir.join("stutter");

    if env::var_os("STUTTER_USE_PREBUILT_BPF").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let object = env::var_os("STUTTER_BPF_OBJECT").ok_or_else(|| {
            anyhow!("STUTTER_USE_PREBUILT_BPF=1 requires STUTTER_BPF_OBJECT to be set")
        })?;

        let object = PathBuf::from(object);

        if !object.exists() {
            return Err(anyhow!(
                "STUTTER_BPF_OBJECT {} does not exist",
                object.display()
            ));
        }

        std::fs::copy(&object, &out_object).with_context(|| {
            format!(
                "failed to copy prebuilt BPF object {} to {}",
                object.display(),
                out_object.display()
            )
        })?;

        return Ok(());
    }

    // Default path: build eBPF locally.
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
