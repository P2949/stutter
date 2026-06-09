use std::{env, path::PathBuf, process::Command};

use anyhow::{Context as _, anyhow};
use aya_build::Toolchain;

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-env-changed=STUTTER_USE_PREBUILT_BPF");
    println!("cargo:rerun-if-env-changed=STUTTER_BPF_OBJECT");
    println!("cargo:rerun-if-env-changed=RUSTUP_TOOLCHAIN");
    emit_build_metadata();

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
    let rustup_toolchain = env::var("RUSTUP_TOOLCHAIN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let toolchain = rustup_toolchain
        .as_deref()
        .map(Toolchain::Custom)
        .unwrap_or_default();

    aya_build::build_ebpf([ebpf_package], toolchain)
}

fn emit_build_metadata() {
    println!("cargo:rerun-if-env-changed=STUTTER_GIT_REV");
    emit_git_rerun_hints();

    let git_rev = env::var("STUTTER_GIT_REV")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(detect_git_revision)
        .unwrap_or_else(|| "unknown".to_owned());

    let package_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_owned());

    println!("cargo:rustc-env=STUTTER_GIT_REV={git_rev}");
    println!("cargo:rustc-env=STUTTER_BUILD_VERSION={package_version} (git {git_rev})");
}

fn emit_git_rerun_hints() {
    let git_dir = PathBuf::from("..").join(".git");
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());

    let Ok(head) = std::fs::read_to_string(&head_path) else {
        return;
    };

    if let Some(reference) = head.strip_prefix("ref: ") {
        let ref_path = git_dir.join(reference.trim());
        println!("cargo:rerun-if-changed={}", ref_path.display());
    }
}

fn detect_git_revision() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn ensure_cargo_bin_on_path() {
    let Some(home) = env::var_os("HOME") else {
        return;
    };

    let cargo_bin = PathBuf::from(home).join(".cargo").join("bin");
    if !cargo_bin.join("rustup").exists() {
        return;
    }

    // Optionally include repository-local wrapper scripts so we can intercept
    // eBPF toolchain executables (like bpf-linker) and filter their output.
    let repo_wrapper = PathBuf::from("..").join("scripts").join("wrappers");

    let current_path = env::var_os("PATH").unwrap_or_default();
    // If PATH already contains cargo_bin and repo_wrapper (when present), nothing to do.
    let mut has_cargo_bin = false;
    let mut has_repo_wrapper = false;
    for p in env::split_paths(&current_path) {
        if p == cargo_bin {
            has_cargo_bin = true;
        }
        if repo_wrapper.exists() && p == repo_wrapper {
            has_repo_wrapper = true;
        }
    }

    if has_cargo_bin && (repo_wrapper.exists() && has_repo_wrapper || !repo_wrapper.exists()) {
        return;
    }

    // Build a new PATH with repo_wrapper (if present) first, then cargo_bin, then
    // the existing PATH entries (excluding duplicates we added).
    let mut paths = Vec::new();
    if repo_wrapper.exists() {
        paths.push(repo_wrapper.clone());
    }
    paths.push(cargo_bin.clone());
    for p in env::split_paths(&current_path) {
        if p == cargo_bin {
            continue;
        }
        if repo_wrapper.exists() && p == repo_wrapper {
            continue;
        }
        paths.push(p);
    }

    if let Ok(path) = env::join_paths(paths) {
        // SAFETY: build scripts are single-threaded here, and this happens before
        // aya-build spawns the cargo/rustup child processes that need the PATH.
        unsafe {
            env::set_var("PATH", path);
        }
    }
}
