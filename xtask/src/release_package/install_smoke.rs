//! `xtask release-install-smoke`: unpack the packaged tarball and exercise the installed binary on
//! the exact recording the candidate's own live observation produced. Every expectation (version,
//! git revision, Supported capability list, eBPF digest) comes from the package's own
//! `release-manifest.json`, so a release does not depend on a validation-campaign manifest. The
//! Core Validation campaign's install-smoke task calls the same implementation.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde_json::Value;

use super::MANIFEST_NAME;
use crate::validation_campaign::{
    repository::{file_sha256, reports_revision},
    shell::{Shell, SystemShell},
};

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct InstallSmokeArgs {
    /// The release directory written by `release-package`.
    #[arg(long)]
    pub package: PathBuf,
    /// The exact-RC observation recording (a directory holding `session.json`).
    #[arg(long)]
    pub recording: PathBuf,
    /// Scratch directory the tarball is unpacked into (replaced; defaults to a temporary one).
    #[arg(long)]
    pub install: Option<PathBuf>,
    /// Also require the package to be for exactly this commit.
    #[arg(long)]
    pub expect_commit: Option<String>,
}

/// What the package itself says it is, read back from the installed binary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmokeFacts {
    pub version: String,
    pub revision: String,
    pub supported: Vec<String>,
}

fn step(
    shell: &dyn Shell,
    binary: &str,
    args: &[&str],
    cwd: &Path,
    what: &str,
) -> Result<String, String> {
    let out = shell.run(binary, args, cwd);
    if out.ok() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "{what} failed ({}): {}",
            out.code,
            out.stderr.lines().last().unwrap_or("")
        ))
    }
}

/// Unpacks `package` into `install` and checks the installed binary, capabilities, doctor, help
/// surfaces, the recording and the eBPF object against `release-manifest.json`.
pub fn smoke(
    shell: &dyn Shell,
    package: &Path,
    recording: &Path,
    install: &Path,
) -> Result<SmokeFacts, String> {
    let manifest: Value = fs::read(package.join(MANIFEST_NAME))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("release-manifest.json is missing or unreadable")?;
    let version = manifest["version"]
        .as_str()
        .ok_or("manifest has no version")?;
    let revision = manifest["git_revision"]
        .as_str()
        .ok_or("manifest has no revision")?;
    let mut expected: Vec<String> = manifest["supported_capabilities"]
        .as_array()
        .ok_or("manifest has no supported_capabilities")?
        .iter()
        .filter_map(|c| c.as_str().map(str::to_owned))
        .collect();
    let _ = fs::remove_dir_all(install);
    fs::create_dir_all(install).map_err(|e| e.to_string())?;
    let tarball = format!("stutter-{version}-linux-x86_64.tar.gz");
    step(
        shell,
        "tar",
        &[
            "--strip-components=1",
            "-xzf",
            &package.join(&tarball).to_string_lossy(),
            "-C",
            &install.to_string_lossy(),
        ],
        install,
        "unpacking the tarball",
    )?;
    let binary = install.join("bin/stutter");
    let bin = binary.to_string_lossy().into_owned();
    let reported = step(shell, &bin, &["--version"], install, "stutter --version")?;
    if !reported.starts_with(&format!("stutter {version} ("))
        || !reports_revision(&reported, revision)
    {
        return Err(format!(
            "the packaged binary reports {:?}, expected version {version} at {revision}",
            reported.trim()
        ));
    }
    let capabilities: Value = serde_json::from_str(&step(
        shell,
        &bin,
        &["capabilities", "--json"],
        install,
        "stutter capabilities",
    )?)
    .map_err(|e| format!("capabilities output: {e}"))?;
    let mut actual: Vec<String> = capabilities
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["tier"] == "supported")
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect();
    expected.sort();
    actual.sort();
    if expected != actual {
        return Err(format!(
            "the packaged binary supports {actual:?}, the release manifest lists {expected:?}"
        ));
    }
    // Doctor must run to a verdict; an unprivileged host may legitimately report failures.
    let doctor = shell.run(&bin, &["doctor", "--json"], install);
    if serde_json::from_str::<Value>(&doctor.stdout).map_or(true, |v| v["overall"].is_null()) {
        return Err("stutter doctor produced no verdict".to_owned());
    }
    for help in [["restore", "--help"], ["audit", "--help"]] {
        step(shell, &bin, &help, install, &format!("stutter {}", help[0]))?;
    }
    if !recording.join("session.json").is_file() {
        return Err("no exact-RC observation recording to validate".to_owned());
    }
    let recording = recording.to_string_lossy().into_owned();
    step(
        shell,
        &bin,
        &["validate", &recording],
        install,
        "stutter validate",
    )?;
    let analysis: Value = serde_json::from_str(&step(
        shell,
        &bin,
        &["report", &recording, "--analysis-json"],
        install,
        "stutter report",
    )?)
    .map_err(|e| format!("report output: {e}"))?;
    if analysis.get("cluster_analysis").is_none() {
        return Err("the report carries no diagnosis analysis".to_owned());
    }
    let object = install.join("lib/stutter/stutter.bpf.o");
    let bytes = fs::read(&object).map_err(|e| format!("packaged eBPF object: {e}"))?;
    if !bytes.starts_with(b"\x7fELF")
        || file_sha256(&object).ok().as_deref() != manifest["bpf_object"]["sha256"].as_str()
    {
        return Err("the packaged eBPF object is not the manifest's ELF object".to_owned());
    }
    Ok(SmokeFacts {
        version: version.to_owned(),
        revision: revision.to_owned(),
        supported: actual,
    })
}

pub fn run(args: &InstallSmokeArgs) -> anyhow::Result<()> {
    let scratch;
    let install = match &args.install {
        Some(path) => path.as_path(),
        None => {
            scratch = std::env::temp_dir().join(format!(
                "stutter-install-smoke-{}-{}",
                std::process::id(),
                crate::evidence_metadata::unix_ms()
            ));
            scratch.as_path()
        }
    };
    let result = smoke(&SystemShell, &args.package, &args.recording, install);
    if args.install.is_none() {
        let _ = fs::remove_dir_all(install);
    }
    match result {
        Ok(facts) => {
            if let Some(commit) = &args.expect_commit
                && facts.revision != *commit
            {
                bail!(
                    "the package is for {}, not the expected commit {commit}",
                    facts.revision
                );
            }
            println!(
                "install_smoke=PASS version={} revision={} supported={}",
                facts.version,
                facts.revision,
                facts.supported.join(",")
            );
            Ok(())
        }
        Err(summary) => bail!("install smoke failed: {summary}"),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::evidence_bundle::tests::Dir;

    const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
    const ELF: &[u8] = b"\x7fELF-fake-bpf-object";

    /// A package whose "binary" is a script answering the way the real one does.
    struct Package {
        dir: Dir,
        recording: Dir,
    }

    #[derive(Clone)]
    struct Spec {
        reported_version: String,
        reported_revision: String,
        capabilities: &'static str,
        bpf: &'static [u8],
        manifest_digest: Option<String>,
        recording: bool,
    }

    fn spec() -> Spec {
        Spec {
            reported_version: "0.2.0".to_owned(),
            reported_revision: REVISION[..7].to_owned(),
            capabilities: r#"[{"id":"bounded_autotune_controller","tier":"supported"},{"id":"doctor","tier":"supported"},{"id":"uclamp_experiment","tier":"experimental"}]"#,
            bpf: ELF,
            manifest_digest: None,
            recording: true,
        }
    }

    fn package(spec: &Spec) -> Package {
        let dir = Dir::new("install-package");
        let tree = Dir::new("install-tree");
        let root = tree.join("stutter-0.2.0-linux-x86_64");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("lib/stutter")).unwrap();
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n\
             --version) echo 'stutter {} (git {})';;\n\
             capabilities) echo '{}';;\n\
             doctor) echo '{{\"overall\":\"warn\"}}';;\n\
             restore|audit) echo help;;\n\
             validate) [ -f \"$2/session.json\" ] || exit 1;;\n\
             report) echo '{{\"cluster_analysis\":{{}}}}';;\n\
             esac\n",
            spec.reported_version, spec.reported_revision, spec.capabilities
        );
        fs::write(root.join("bin/stutter"), script).unwrap();
        fs::set_permissions(root.join("bin/stutter"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("lib/stutter/stutter.bpf.o"), spec.bpf).unwrap();
        let tarball = dir.join("stutter-0.2.0-linux-x86_64.tar.gz");
        assert!(
            std::process::Command::new("tar")
                .args(["-czf"])
                .arg(&tarball)
                .arg("-C")
                .arg(&tree.0)
                .arg("stutter-0.2.0-linux-x86_64")
                .status()
                .unwrap()
                .success()
        );
        let digest = spec
            .manifest_digest
            .clone()
            .unwrap_or_else(|| file_sha256(&root.join("lib/stutter/stutter.bpf.o")).unwrap());
        fs::write(
            dir.join(MANIFEST_NAME),
            serde_json::to_vec(&json!({
                "version": "0.2.0",
                "git_revision": REVISION,
                "supported_capabilities": ["bounded_autotune_controller", "doctor"],
                "bpf_object": {"sha256": digest},
            }))
            .unwrap(),
        )
        .unwrap();
        let recording = Dir::new("install-recording");
        if spec.recording {
            fs::write(recording.join("session.json"), "{}").unwrap();
        }
        Package { dir, recording }
    }

    fn smoke_of(spec: &Spec) -> Result<SmokeFacts, String> {
        let package = package(spec);
        let install = Dir::new("install-out");
        smoke(
            &SystemShell,
            &package.dir.0,
            &package.recording.0,
            &install.join("x"),
        )
    }

    #[test]
    fn a_correct_package_passes_and_reports_what_the_manifest_says() {
        let facts = smoke_of(&spec()).unwrap();
        assert_eq!(facts.version, "0.2.0");
        assert_eq!(facts.revision, REVISION);
        assert_eq!(facts.supported, ["bounded_autotune_controller", "doctor"]);
    }

    #[test]
    fn a_wrong_version_or_revision_is_refused() {
        let wrong_version = Spec {
            reported_version: "0.1.0".to_owned(),
            ..spec()
        };
        assert!(smoke_of(&wrong_version).unwrap_err().contains("reports"));
        let wrong_revision = Spec {
            reported_revision: "fedcba9".to_owned(),
            ..spec()
        };
        assert!(smoke_of(&wrong_revision).unwrap_err().contains("reports"));
    }

    #[test]
    fn a_capability_mismatch_is_refused() {
        let extra = Spec {
            capabilities: r#"[{"id":"bounded_autotune_controller","tier":"supported"},{"id":"doctor","tier":"supported"},{"id":"nice_experiment","tier":"supported"}]"#,
            ..spec()
        };
        assert!(smoke_of(&extra).unwrap_err().contains("supports"));
        let missing = Spec {
            capabilities: r#"[{"id":"doctor","tier":"supported"}]"#,
            ..spec()
        };
        assert!(smoke_of(&missing).unwrap_err().contains("supports"));
    }

    #[test]
    fn a_missing_or_invalid_recording_is_refused() {
        let missing = Spec {
            recording: false,
            ..spec()
        };
        assert!(smoke_of(&missing).unwrap_err().contains("recording"));
    }

    #[test]
    fn a_non_elf_object_or_a_digest_mismatch_is_refused() {
        let not_elf = Spec {
            bpf: b"not an elf",
            ..spec()
        };
        assert!(smoke_of(&not_elf).unwrap_err().contains("eBPF"));
        let wrong_digest = Spec {
            manifest_digest: Some("0".repeat(64)),
            ..spec()
        };
        assert!(smoke_of(&wrong_digest).unwrap_err().contains("eBPF"));
    }

    #[test]
    fn a_package_without_a_manifest_is_refused() {
        let package = package(&spec());
        fs::remove_file(package.dir.join(MANIFEST_NAME)).unwrap();
        let install = Dir::new("install-out");
        let error = smoke(
            &SystemShell,
            &package.dir.0,
            &package.recording.0,
            &install.join("x"),
        )
        .unwrap_err();
        assert!(error.contains("release-manifest.json"));
    }
}
