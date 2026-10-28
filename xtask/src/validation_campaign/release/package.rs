//! `release-package`, `release-verify` and `install-smoke`: the existing release implementation
//! is driven, never duplicated. Smoke-testing uses the packaged artifact itself, on the exact
//! recording the candidate's own live observation produced.
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{blocked, failed, release_dir};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::TaskSpec,
    model::FailureClass,
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
    name.split('-')
        .any(|token| token.len() >= 7 && commit.starts_with(token))
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

impl InstallSmokeRunner {
    fn smoke(&self, context: &RunContext<'_>) -> Result<(), String> {
        let recording = observation_recording(&context.dir.0)
            .ok_or("no exact-RC observation recording to validate")?;
        let facts = crate::release_package::install_smoke::smoke(
            self.shell.as_ref(),
            &context.dir.0.join("release/package"),
            &recording,
            &context.dir.0.join("release/install"),
        )?;
        if Some(facts.revision.as_str()) != context.state.release_candidate.as_deref()
            || facts.version != context.manifest.release.version
        {
            return Err(
                "the package does not certify the frozen release candidate and manifest version"
                    .to_owned(),
            );
        }
        let mut expected = context.manifest.policy.capabilities.supported.clone();
        let mut actual = facts.supported;
        expected.sort();
        actual.sort();
        if expected != actual {
            return Err(format!(
                "the packaged binary supports {actual:?}, the policy requires {expected:?}"
            ));
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
