//! Independent release verifier: recomputes digests, unpacks the artifacts and checks
//! them against the manifest, the source archive and the capability registry.
use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, ensure};
use serde_json::Value;
use stutter::api::capabilities::{Capability, CapabilityTier, capability_registry};

use super::{
    MANIFEST_NAME, RELEASE_SCHEMA_VERSION, SBOM_NAME, SUMS_NAME, SUPPORT_NAME, VerifyArgs,
    acceptance::check_acceptance, file_sha256, layout_entries, parse_binary_version, run,
};
use crate::evidence_bundle::{is_hex, sha256::sha256_hex};

/// The support statement shipped with a release; regenerated and compared on verify.
pub(super) fn support_text(version: &str, commit: &str, capabilities: &[Capability]) -> String {
    let mut text =
        format!("# Stutter {version} support statement\n\nSource revision: `{commit}`\n\n");
    for (tier, title) in [
        (CapabilityTier::Supported, "Supported"),
        (CapabilityTier::Experimental, "Experimental"),
        (CapabilityTier::Internal, "Internal"),
        (CapabilityTier::Disabled, "Disabled"),
    ] {
        text.push_str(&format!("## {title}\n\n"));
        let mut any = false;
        for capability in capabilities.iter().filter(|c| c.tier == tier) {
            any = true;
            text.push_str(&format!(
                "- `{}`: {} - {}\n",
                capability.id, capability.title, capability.limitations
            ));
        }
        if !any {
            text.push_str("- none\n");
        }
        text.push('\n');
    }
    text.push_str("A capability is Supported only when its required live evidence is verified for this exact revision.\n");
    text
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> anyhow::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "stutter-release-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unpack(archive: &Path, into: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(into)?;
    run(into, "tar", &["-xzf", &archive.to_string_lossy()])?;
    Ok(())
}

pub fn verify_release(args: &VerifyArgs) -> anyhow::Result<()> {
    let dir = fs::canonicalize(&args.dir)?;
    let manifest: Value = serde_json::from_slice(&fs::read(dir.join(MANIFEST_NAME))?)?;
    ensure!(
        manifest["schema_version"] == RELEASE_SCHEMA_VERSION,
        "unsupported release manifest schema"
    );
    let version = manifest["version"].as_str().context("version")?;
    let commit = manifest["git_revision"].as_str().context("git_revision")?;
    ensure!(is_hex(commit, 40), "git_revision is not a full commit id");
    ensure!(
        is_hex(manifest["cargo_lock_sha256"].as_str().unwrap_or(""), 64),
        "cargo_lock_sha256"
    );

    // 1. SHA256SUMS covers exactly the published files and every digest matches.
    let sums = fs::read_to_string(dir.join(SUMS_NAME))?;
    let mut listed = BTreeSet::new();
    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
        let (hash, name) = line.split_once("  ").context("malformed SHA256SUMS line")?;
        ensure!(
            is_hex(hash, 64) && !name.contains('/'),
            "bad SHA256SUMS entry {line}"
        );
        ensure!(
            file_sha256(&dir.join(name))? == hash,
            "digest mismatch for {name}"
        );
        listed.insert(name.to_owned());
    }
    let tarball = format!("stutter-{version}-linux-x86_64.tar.gz");
    let source = format!("stutter-{version}-source.tar.gz");
    let expected = [
        tarball.as_str(),
        source.as_str(),
        SBOM_NAME,
        SUPPORT_NAME,
        MANIFEST_NAME,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    ensure!(
        listed == expected,
        "SHA256SUMS lists {listed:?}, expected {expected:?}"
    );
    for artifact in manifest["artifacts"].as_array().context("artifacts")? {
        let name = artifact["name"].as_str().context("artifact name")?;
        ensure!(
            file_sha256(&dir.join(name))? == artifact["sha256"].as_str().unwrap_or(""),
            "manifest digest differs for {name}"
        );
    }

    // 2. Unpack the source archive and the package.
    let scratch = Scratch::new()?;
    unpack(&dir.join(&source), &scratch.0.join("source"))?;
    let source_root = scratch.0.join("source").join(format!("stutter-{version}"));
    ensure!(
        source_root.join("Cargo.toml").is_file(),
        "source archive lacks the stutter-{version}/ root"
    );
    ensure!(
        file_sha256(&source_root.join("Cargo.lock"))?
            == manifest["cargo_lock_sha256"].as_str().unwrap_or(""),
        "Cargo.lock digest differs between the manifest and the source archive"
    );
    unpack(&dir.join(&tarball), &scratch.0.join("package"))?;
    let package_root = scratch
        .0
        .join("package")
        .join(format!("stutter-{version}-linux-x86_64"));
    let layout = fs::read_to_string(source_root.join("packaging/tarball/MANIFEST.txt"))?;
    for entry in layout_entries(&layout) {
        ensure!(package_root.join(entry).exists(), "package lacks {entry}");
    }
    let packaged = manifest["packaged_files"]
        .as_array()
        .context("packaged_files")?;
    for file in packaged {
        let path = file["path"].as_str().context("path")?;
        ensure!(
            file_sha256(&package_root.join(path))? == file["sha256"].as_str().unwrap_or(""),
            "packaged digest differs for {path}"
        );
    }

    // 3. Binary and the required eBPF object.
    let binary = package_root.join("bin/stutter");
    ensure!(
        file_sha256(&binary)? == manifest["binary_sha256"].as_str().unwrap_or(""),
        "binary digest differs from the manifest"
    );
    let (binary_version, git_short) = parse_binary_version(&run(
        &package_root,
        &binary.to_string_lossy(),
        &["--version"],
    )?)?;
    ensure!(
        binary_version == version && commit.starts_with(&git_short),
        "packaged binary reports {binary_version} (git {git_short}); manifest says {version} at {commit}"
    );
    let object = fs::read(package_root.join("lib/stutter/stutter.bpf.o"))?;
    ensure!(
        object.starts_with(b"\x7fELF") && object.len() > 64,
        "packaged eBPF object is empty or not ELF"
    );
    ensure!(
        manifest["bpf_object"]["required_in_package"] == true
            && sha256_hex(&object) == manifest["bpf_object"]["sha256"].as_str().unwrap_or(""),
        "eBPF object digest or requirement differs from the manifest"
    );

    // 4. Support statement and claims never exceed the registry.
    let capabilities = capability_registry();
    let registry = capabilities
        .iter()
        .filter(|c| c.tier == CapabilityTier::Supported)
        .map(|c| c.id.to_owned())
        .collect::<BTreeSet<_>>();
    let claimed = manifest["supported_capabilities"]
        .as_array()
        .context("supported_capabilities")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    ensure!(
        claimed == registry,
        "release claims {claimed:?} but the registry supports {registry:?}"
    );
    ensure!(
        fs::read_to_string(dir.join(SUPPORT_NAME))? == support_text(version, commit, capabilities),
        "support statement differs from the capability registry"
    );

    // 5. SBOM covers the lockfile.
    let sbom: Value = serde_json::from_slice(&fs::read(dir.join(SBOM_NAME))?)?;
    let lock = fs::read_to_string(source_root.join("Cargo.lock"))?;
    ensure!(
        sbom["components"].as_array().map(Vec::len) == Some(lock.matches("[[package]]").count()),
        "SBOM does not cover every Cargo.lock package"
    );

    // 6. Optional: external verified acceptance bundles for exactly this commit. The
    // committed evidence index is historical and is deliberately not consulted.
    if args.require_acceptance {
        let accepted = check_acceptance(capabilities, commit, &args.acceptance_bundle)?;
        println!("acceptance_suites={accepted:?}");
    } else {
        ensure!(
            args.acceptance_bundle.is_empty(),
            "--acceptance-bundle requires --require-acceptance"
        );
    }
    println!("release_verified={}", dir.display());
    Ok(())
}
