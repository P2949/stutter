//! Release contract: package the supported layout, bind it to the exact source commit,
//! and verify it independently. The packaged eBPF object is REQUIRED (it is listed in
//! `packaging/tarball/MANIFEST.txt`); the binary embeds an identical copy and
//! `STUTTER_BPF_OBJECT` is only a runtime override. Absence is a release failure.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail, ensure};
use serde_json::{Value, json};

use crate::evidence_bundle::sha256::sha256_hex;

mod sbom;
mod verify;

pub use verify::verify_release;

pub(crate) const RELEASE_SCHEMA_VERSION: u64 = 1;
pub(crate) const MANIFEST_NAME: &str = "release-manifest.json";
pub(crate) const SUMS_NAME: &str = "SHA256SUMS";
pub(crate) const SBOM_NAME: &str = "sbom.cdx.json";
pub(crate) const SUPPORT_NAME: &str = "SUPPORT.md";

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct PackageArgs {
    /// Release binary built from the committed source.
    #[arg(long)]
    pub binary: PathBuf,
    /// eBPF object located in the build output (required in the package).
    #[arg(long)]
    pub bpf_object: PathBuf,
    /// New output directory.
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct VerifyArgs {
    pub dir: PathBuf,
    /// Also require every live suite needed by Supported capabilities to have a PASS
    /// entry in docs/evidence/index.json for exactly this release's commit.
    #[arg(long)]
    pub require_acceptance: bool,
}

pub(crate) fn run(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("spawn {program}"))?;
    ensure!(
        output.status.success(),
        "{program} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) fn file_sha256(path: &Path) -> anyhow::Result<String> {
    Ok(sha256_hex(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    ))
}

/// `stutter --version` is `stutter <version> (git <short>)`.
pub(crate) fn parse_binary_version(text: &str) -> anyhow::Result<(String, String)> {
    let rest = text
        .strip_prefix("stutter ")
        .context("unexpected --version")?;
    let (version, tail) = rest
        .split_once(' ')
        .context("version has no git revision")?;
    let git = tail
        .trim()
        .strip_prefix("(git ")
        .and_then(|tail| tail.strip_suffix(')'))
        .context("version has no git revision")?;
    Ok((version.to_owned(), git.to_owned()))
}

fn layout_entries(manifest: &str) -> Vec<&str> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Source for a packaged path, relative to the repository (None for generated paths).
fn repo_source(entry: &str) -> Option<String> {
    if let Some(name) = entry.strip_prefix("share/doc/stutter/") {
        return Some(format!("docs/{name}"));
    }
    if let Some(name) = entry.strip_prefix("share/stutter/systemd/") {
        return Some(format!("packaging/systemd/{name}"));
    }
    entry
        .strip_prefix("share/stutter/openrc/")
        .map(|name| format!("packaging/openrc/{name}"))
}

fn gzip_deterministic(path: &Path) -> anyhow::Result<PathBuf> {
    let status = Command::new("gzip")
        .args(["-n", "-f", "-9"])
        .arg(path)
        .status()?;
    ensure!(status.success(), "gzip failed");
    Ok(PathBuf::from(format!("{}.gz", path.display())))
}

pub fn package_release(root: &Path, args: &PackageArgs) -> anyhow::Result<()> {
    ensure!(!args.out.exists(), "{} already exists", args.out.display());
    ensure!(
        run(root, "git", &["status", "--porcelain"])?.is_empty(),
        "release packaging requires a clean committed tree"
    );
    let commit = run(root, "git", &["rev-parse", "HEAD"])?;
    let commit_time = run(root, "git", &["log", "-1", "--format=%ct"])?;
    let (version, git_short) =
        parse_binary_version(&run(root, &args.binary.to_string_lossy(), &["--version"])?)?;
    ensure!(
        commit.starts_with(&git_short),
        "binary reports git {git_short} but the source is {commit}; rebuild from this commit"
    );
    let bpf = fs::read(&args.bpf_object).with_context(|| {
        "the eBPF object is required in the package (lib/stutter/stutter.bpf.o); \
         a missing object is a release failure"
    })?;
    ensure!(
        bpf.len() > 64 && bpf.starts_with(b"\x7fELF"),
        "eBPF object is empty or not an ELF file"
    );

    fs::create_dir_all(&args.out)?;
    let out = fs::canonicalize(&args.out)?;
    let name = format!("stutter-{version}-linux-x86_64");
    let stage = out.join(&name);
    let layout = fs::read_to_string(root.join("packaging/tarball/MANIFEST.txt"))?;
    let mut packaged = Vec::new();
    for entry in layout_entries(&layout) {
        let target = stage.join(entry);
        if entry.ends_with('/') {
            fs::create_dir_all(&target)?;
            continue;
        }
        fs::create_dir_all(target.parent().context("no parent")?)?;
        match entry {
            "bin/stutter" => fs::copy(&args.binary, &target).map(|_| ())?,
            "lib/stutter/stutter.bpf.o" => fs::write(&target, &bpf)?,
            other => {
                let source = repo_source(other)
                    .with_context(|| format!("unhandled layout entry {other}"))?;
                fs::copy(root.join(&source), &target)
                    .with_context(|| format!("missing packaged file {source}"))?;
            }
        }
        packaged.push(json!({"path":entry,"sha256":file_sha256(&target)?}));
    }

    // Reproducible archives: fixed order, owner and mtime (the commit time).
    let tarball = out.join(format!("{name}.tar"));
    let status = Command::new("tar")
        .current_dir(&out)
        .args(["--sort=name", "--owner=0", "--group=0", "--numeric-owner"])
        .arg(format!("--mtime=@{commit_time}"))
        .arg("-cf")
        .arg(&tarball)
        .arg(&name)
        .status()?;
    ensure!(status.success(), "tar failed");
    let tarball = gzip_deterministic(&tarball)?;
    let source_tar = out.join(format!("stutter-{version}-source.tar"));
    let status = Command::new("git")
        .current_dir(root)
        .args([
            "archive",
            "--format=tar",
            &format!("--prefix=stutter-{version}/"),
            "-o",
        ])
        .arg(&source_tar)
        .arg("HEAD")
        .status()?;
    ensure!(status.success(), "git archive failed");
    let source_tar = gzip_deterministic(&source_tar)?;
    fs::remove_dir_all(&stage)?;

    let lock = fs::read_to_string(root.join("Cargo.lock"))?;
    fs::write(
        out.join(SBOM_NAME),
        sbom::cyclonedx_from_lock(&lock, &version)?,
    )?;
    let capabilities = stutter::api::capabilities::capability_registry();
    let supported = capabilities
        .iter()
        .filter(|c| c.tier == stutter::api::capabilities::CapabilityTier::Supported)
        .map(|c| c.id)
        .collect::<Vec<_>>();
    fs::write(
        out.join(SUPPORT_NAME),
        verify::support_text(&version, &commit, capabilities),
    )?;
    let file = |path: &Path| -> anyhow::Result<Value> {
        Ok(json!({"name":path.file_name().and_then(|n| n.to_str()),"sha256":file_sha256(path)?}))
    };
    let manifest = json!({
        "schema_version": RELEASE_SCHEMA_VERSION,
        "version": version,
        "git_revision": commit,
        "cargo_lock_sha256": file_sha256(&root.join("Cargo.lock"))?,
        "binary_sha256": file_sha256(&args.binary)?,
        "bpf_object": {
            "sha256": sha256_hex(&bpf),
            "required_in_package": true,
            "semantics": "packaged at lib/stutter/stutter.bpf.o; the binary embeds an identical object; STUTTER_BPF_OBJECT is a runtime override",
        },
        "artifact_compatibility": {
            "recording_session_schema": stutter::api::recorder::SESSION_SCHEMA_VERSION.get(),
            "controller_journal_schema": stutter::api::autotune::controller_journal::CONTROLLER_JOURNAL_SCHEMA_VERSION,
            "evidence_bundle_schema": crate::evidence_bundle::BUNDLE_SCHEMA_VERSION,
            "release_manifest_schema": RELEASE_SCHEMA_VERSION,
        },
        "supported_capabilities": supported,
        "support_statement": SUPPORT_NAME,
        "packaged_files": packaged,
        "artifacts": [file(&tarball)?, file(&source_tar)?, file(&out.join(SBOM_NAME))?, file(&out.join(SUPPORT_NAME))?],
        "sbom": SBOM_NAME,
    });
    fs::write(
        out.join(MANIFEST_NAME),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let mut sums = String::new();
    for path in [
        &tarball,
        &source_tar,
        &out.join(SBOM_NAME),
        &out.join(SUPPORT_NAME),
        &out.join(MANIFEST_NAME),
    ] {
        let name = path.file_name().and_then(|n| n.to_str()).context("name")?;
        sums.push_str(&format!("{}  {name}\n", file_sha256(path)?));
    }
    let mut sums_sorted = sums.lines().collect::<Vec<_>>();
    sums_sorted.sort_by_key(|line| line.split("  ").nth(1).unwrap_or("").to_owned());
    fs::write(out.join(SUMS_NAME), format!("{}\n", sums_sorted.join("\n")))?;
    println!("release_packaged={}", out.display());
    if bail_if_unsupported_claims(&manifest).is_err() {
        bail!("manifest claims capabilities beyond the registry");
    }
    Ok(())
}

fn bail_if_unsupported_claims(manifest: &Value) -> anyhow::Result<()> {
    let registry = stutter::api::capabilities::capability_registry()
        .iter()
        .filter(|c| c.tier == stutter::api::capabilities::CapabilityTier::Supported)
        .map(|c| c.id.to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let claimed = manifest["supported_capabilities"]
        .as_array()
        .context("supported_capabilities")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        claimed == registry,
        "supported capability list differs from the registry"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
