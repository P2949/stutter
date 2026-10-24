//! `release-package`, `release-verify` and `install-smoke`: the existing release implementation
//! is driven, never duplicated. Smoke-testing uses the packaged artifact itself, on the exact
//! recording the candidate's own live observation produced.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{blocked, failed, release_dir};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::TaskSpec,
    model::FailureClass,
    repository::file_sha256,
    shell::Shell,
};

pub const RC_LIVE_TASKS: [&str; 4] = [
    "rc-live-observation",
    "rc-game-runnable",
    "rc-affinity",
    "rc-recovery",
];

/// The newest `stutter-*/out/stutter` under the release build directory, the same object the
/// release workflow locates.
pub fn find_ebpf_object(release_build: &Path) -> Option<PathBuf> {
    fs::read_dir(release_build)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("stutter-"))
        .map(|e| e.path().join("out/stutter"))
        .filter(|p| p.is_file())
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// Bundle directories are named `<suite>-<short commit>-<timestamp>`; an earlier candidate's
/// bundles stay on disk as history and must never certify a later one.
pub fn is_candidate_bundle(name: &str, commit: &str) -> bool {
    commit.len() >= 7 && name.contains(&format!("-{}-", &commit[..7]))
}

/// Published bundle directories of the current candidate's live tasks, in a stable order.
pub fn rc_bundles(context: &RunContext<'_>) -> Vec<PathBuf> {
    let commit = context.state.release_candidate.clone().unwrap_or_default();
    let mut bundles = Vec::new();
    for task in RC_LIVE_TASKS {
        let mut dirs: Vec<PathBuf> = fs::read_dir(context.dir.bundles().join(task))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| is_candidate_bundle(&n.to_string_lossy(), &commit))
            })
            .collect();
        dirs.sort();
        bundles.extend(dirs);
    }
    bundles
}

pub struct ReleasePackageRunner {
    pub shell: Box<dyn Shell>,
}

impl TaskRunner for ReleasePackageRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let repo = context.repository;
        let build = self
            .shell
            .run("cargo", &["build", "-p", "stutter", "--release"], repo);
        if !build.ok() {
            return failed(
                FailureClass::SoftwareDefect,
                "build",
                "the release build failed",
            );
        }
        let Some(object) = find_ebpf_object(&repo.join("target/release/build")) else {
            return failed(
                FailureClass::SoftwareDefect,
                "ebpf-object",
                "the eBPF object was not found in the build output; refusing to release",
            );
        };
        let Ok(dir) = release_dir(&context.dir.0) else {
            return failed(
                FailureClass::Retryable,
                "storage",
                "cannot create the release directory",
            );
        };
        let package = dir.join("package");
        let _ = fs::remove_dir_all(&package);
        let out = self.shell.run(
            &context.binaries.xtask.to_string_lossy(),
            &[
                "release-package",
                "--binary",
                &repo.join("target/release/stutter").to_string_lossy(),
                "--bpf-object",
                &object.to_string_lossy(),
                "--out",
                &package.to_string_lossy(),
            ],
            repo,
        );
        if !out.ok() {
            return failed(
                FailureClass::SoftwareDefect,
                "package",
                format!(
                    "release-package failed: {}",
                    out.stderr.lines().last().unwrap_or("")
                ),
            );
        }
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&package)),
            receipt: None,
            limitations: Vec::new(),
            fingerprint: Box::default(),
            updates: Vec::new(),
        }
    }
}

pub struct ReleaseVerifyRunner {
    pub shell: Box<dyn Shell>,
}

impl TaskRunner for ReleaseVerifyRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let package = context.dir.0.join("release/package");
        let bundles = rc_bundles(context);
        if bundles.len() < RC_LIVE_TASKS.len() {
            return failed(
                FailureClass::SoftwareDefect,
                "acceptance",
                format!(
                    "{} exact-RC bundles are published; {} are required",
                    bundles.len(),
                    RC_LIVE_TASKS.len()
                ),
            );
        }
        let mut args = vec![
            "release-verify".to_owned(),
            package.to_string_lossy().into_owned(),
            "--require-acceptance".to_owned(),
        ];
        for bundle in &bundles {
            args.push("--acceptance-bundle".to_owned());
            args.push(bundle.to_string_lossy().into_owned());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.shell.run(
            &context.binaries.xtask.to_string_lossy(),
            &refs,
            context.repository,
        );
        if !out.ok() {
            return failed(
                FailureClass::SoftwareDefect,
                "verify",
                format!(
                    "release-verify failed: {}",
                    out.stderr.lines().last().unwrap_or("")
                ),
            );
        }
        RunnerOutcome::passed()
    }
}

pub struct InstallSmokeRunner {
    pub shell: Box<dyn Shell>,
}

/// A directory holding a `session.json` under the candidate's observation attempts.
pub fn observation_recording(campaign: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if dir.join("session.json").is_file() {
            return Some(dir.to_path_buf());
        }
        if depth == 0 {
            return None;
        }
        let mut children: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        children.sort();
        // Newest attempt first: earlier attempts may belong to an earlier candidate.
        children
            .iter()
            .rev()
            .find_map(|child| walk(child, depth - 1))
    }
    walk(&campaign.join("attempts/rc-live-observation"), 6)
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

impl InstallSmokeRunner {
    fn smoke(&self, context: &RunContext<'_>) -> Result<(), String> {
        let package = context.dir.0.join("release/package");
        let manifest: Value = fs::read(package.join("release-manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or("release-manifest.json is missing or unreadable")?;
        let version = manifest["version"]
            .as_str()
            .ok_or("manifest has no version")?;
        let revision = manifest["git_revision"]
            .as_str()
            .ok_or("manifest has no revision")?;
        if Some(revision) != context.state.release_candidate.as_deref()
            || version != context.manifest.release.version
        {
            return Err(
                "the package does not certify the frozen release candidate and manifest version"
                    .to_owned(),
            );
        }
        let install = context.dir.0.join("release/install");
        let _ = fs::remove_dir_all(&install);
        fs::create_dir_all(&install).map_err(|e| e.to_string())?;
        let tarball = format!("stutter-{version}-linux-x86_64.tar.gz");
        step(
            self.shell.as_ref(),
            "tar",
            &[
                "-xzf",
                "--strip-components=1",
                &package.join(&tarball).to_string_lossy(),
                "-C",
                &install.to_string_lossy(),
            ],
            &install,
            "unpacking the tarball",
        )?;
        let binary = install.join("bin/stutter");
        let bin = binary.to_string_lossy().into_owned();
        let shell = self.shell.as_ref();
        let reported = step(shell, &bin, &["--version"], &install, "stutter --version")?;
        if !reported.contains(&format!("stutter {version} (git {})", &revision[..7])) {
            return Err(format!(
                "the packaged binary reports {:?}, expected version {version} at {revision}",
                reported.trim()
            ));
        }
        let capabilities: Value = serde_json::from_str(&step(
            shell,
            &bin,
            &["capabilities", "--json"],
            &install,
            "stutter capabilities",
        )?)
        .map_err(|e| format!("capabilities output: {e}"))?;
        let supported: Vec<String> = capabilities
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["tier"] == "supported")
            .filter_map(|c| c["id"].as_str().map(str::to_owned))
            .collect();
        let mut expected = context.manifest.policy.capabilities.supported.clone();
        let mut actual = supported;
        expected.sort();
        actual.sort();
        if expected != actual {
            return Err(format!(
                "the packaged binary supports {actual:?}, the policy requires {expected:?}"
            ));
        }
        // Doctor must run to a verdict; an unprivileged host may legitimately report failures.
        let doctor = shell.run(&bin, &["doctor", "--json"], &install);
        if serde_json::from_str::<Value>(&doctor.stdout).map_or(true, |v| v["overall"].is_null()) {
            return Err("stutter doctor produced no verdict".to_owned());
        }
        for help in [["restore", "--help"], ["audit", "--help"]] {
            step(
                shell,
                &bin,
                &help,
                &install,
                &format!("stutter {}", help[0]),
            )?;
        }
        let recording = observation_recording(&context.dir.0)
            .ok_or("no exact-RC observation recording to validate")?;
        let recording = recording.to_string_lossy().into_owned();
        step(
            shell,
            &bin,
            &["validate", &recording],
            &install,
            "stutter validate",
        )?;
        let analysis: Value = serde_json::from_str(&step(
            shell,
            &bin,
            &["report", &recording, "--analysis-json"],
            &install,
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
        Ok(())
    }
}

impl TaskRunner for InstallSmokeRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        if context.state.release_candidate.is_none() {
            return blocked(
                FailureClass::BlockedResource,
                "no release candidate has been frozen",
            );
        }
        match self.smoke(context) {
            Ok(()) => RunnerOutcome::passed(),
            Err(summary) => failed(FailureClass::SoftwareDefect, "install-smoke", summary),
        }
    }
}
