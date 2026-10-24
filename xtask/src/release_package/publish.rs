//! `xtask release-publish`: the last, idempotent step of an authoritative release. The package is
//! verified independently (with externally supplied exact-commit acceptance bundles) before any
//! GitHub mutation; the tag must resolve to the manifest's commit locally and on origin; an
//! existing release is completed, never overwritten.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde_json::Value;

use super::{MANIFEST_NAME, SBOM_NAME, SUMS_NAME, SUPPORT_NAME, VerifyArgs, file_sha256};
use crate::{
    evidence_bundle::is_hex,
    validation_campaign::shell::{Shell, SystemShell},
};

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct PublishArgs {
    /// Release directory produced by `release-package`.
    pub dir: PathBuf,
    /// Exported live-acceptance bundle for exactly this release's commit (repeatable, required).
    #[arg(long = "acceptance-bundle", value_name = "BUNDLE", required = true)]
    pub acceptance_bundle: Vec<PathBuf>,
    /// Release notes (Markdown).
    #[arg(long, value_name = "FILE")]
    pub notes: PathBuf,
}

/// Independent package verification; injectable so publication logic is tested without a package.
pub type Verifier = dyn Fn(&VerifyArgs) -> anyhow::Result<()>;

/// The remote asset names a release must carry.
pub fn asset_names(version: &str) -> Vec<String> {
    vec![
        format!("stutter-{version}-linux-x86_64.tar.gz"),
        format!("stutter-{version}-source.tar.gz"),
        SUMS_NAME.to_owned(),
        MANIFEST_NAME.to_owned(),
        SBOM_NAME.to_owned(),
        SUPPORT_NAME.to_owned(),
    ]
}

pub fn publish_release(repo: &Path, args: &PublishArgs) -> anyhow::Result<()> {
    publish_with(
        &SystemShell,
        &super::verify_release,
        repo,
        &fs::canonicalize(&args.dir)?,
        args,
    )
}

pub fn publish_with(
    shell: &dyn Shell,
    verify: &Verifier,
    repo: &Path,
    dir: &Path,
    args: &PublishArgs,
) -> anyhow::Result<()> {
    ensure!(
        !args.acceptance_bundle.is_empty(),
        "publication requires exact-commit acceptance bundles (--acceptance-bundle)"
    );
    ensure!(args.notes.is_file(), "release notes file not found");
    verify(&VerifyArgs {
        dir: dir.to_path_buf(),
        require_acceptance: true,
        acceptance_bundle: args.acceptance_bundle.clone(),
    })
    .context("package verification failed; nothing was published")?;

    let manifest: Value = serde_json::from_slice(&fs::read(dir.join(MANIFEST_NAME))?)?;
    let version = manifest["version"].as_str().context("manifest version")?;
    let commit = manifest["git_revision"]
        .as_str()
        .context("manifest git_revision")?;
    ensure!(
        is_hex(commit, 40),
        "manifest git_revision is not a commit id"
    );
    let tag = format!("v{version}");
    let assets = asset_names(version);
    let local = check_sums(dir, &assets)?;
    check_tag(shell, repo, &tag, commit)?;

    match remote_release(shell, repo, &tag)? {
        None => create(shell, repo, dir, &tag, version, &assets, &args.notes)?,
        Some(release) => complete(shell, repo, dir, &tag, &assets, &local, &release)?,
    }
    // The published state must now match the verified files exactly.
    let release = remote_release(shell, repo, &tag)?.context("the release is not visible")?;
    for name in &assets {
        let remote = remote_digest(shell, repo, &tag, &release, name)?
            .with_context(|| format!("asset {name} is missing after publication"))?;
        ensure!(
            local.get(name) == Some(&remote),
            "asset {name} differs from the verified file"
        );
    }
    println!("release_published={tag}");
    Ok(())
}

type Digests = std::collections::BTreeMap<String, String>;

/// Every asset exists and every `SHA256SUMS` entry matches its file. Returns each asset's digest.
fn check_sums(dir: &Path, assets: &[String]) -> anyhow::Result<Digests> {
    let sums = fs::read_to_string(dir.join(SUMS_NAME))?;
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, name) = line.split_once("  ").context("malformed SHA256SUMS line")?;
        ensure!(
            file_sha256(&dir.join(name))? == digest,
            "{name} does not match SHA256SUMS"
        );
    }
    let mut digests = Digests::new();
    for name in assets {
        let path = dir.join(name);
        ensure!(path.is_file(), "release asset {name} is missing");
        digests.insert(name.clone(), file_sha256(&path)?);
    }
    Ok(digests)
}

fn check_tag(shell: &dyn Shell, repo: &Path, tag: &str, commit: &str) -> anyhow::Result<()> {
    let local = shell.run(
        "git",
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/tags/{tag}^{{commit}}"),
        ],
        repo,
    );
    ensure!(
        local.ok() && local.stdout.trim() == commit,
        "local tag {tag} does not resolve to {commit}"
    );
    let remote = shell.run(
        "git",
        &[
            "ls-remote",
            "--tags",
            "origin",
            &format!("refs/tags/{tag}"),
            &format!("refs/tags/{tag}^{{}}"),
        ],
        repo,
    );
    ensure!(remote.ok(), "cannot read tags from origin");
    // An annotated tag lists the tag object and its peeled commit; the peeled line wins.
    let mut resolved = None;
    for line in remote.stdout.lines() {
        if let Some((id, name)) = line.split_once('\t')
            && (name.ends_with("^{}") || resolved.is_none())
        {
            resolved = Some(id.to_owned());
        }
    }
    ensure!(
        resolved.as_deref() == Some(commit),
        "origin tag {tag} does not resolve to {commit}"
    );
    Ok(())
}

fn remote_release(shell: &dyn Shell, repo: &Path, tag: &str) -> anyhow::Result<Option<Value>> {
    let out = shell.run(
        "gh",
        &["release", "view", tag, "--json", "tagName,isDraft,assets"],
        repo,
    );
    if !out.ok() {
        if out.stderr.contains("release not found") {
            return Ok(None);
        }
        bail!("cannot read release {tag}: {}", out.stderr.trim());
    }
    Ok(Some(serde_json::from_str(&out.stdout)?))
}

fn create(
    shell: &dyn Shell,
    repo: &Path,
    dir: &Path,
    tag: &str,
    version: &str,
    assets: &[String],
    notes: &Path,
) -> anyhow::Result<()> {
    let title = format!("Stutter v{version}");
    let notes = notes.to_string_lossy().into_owned();
    let paths: Vec<String> = assets
        .iter()
        .map(|name| dir.join(name).to_string_lossy().into_owned())
        .collect();
    let mut argv = vec![
        "release",
        "create",
        tag,
        "--verify-tag",
        "--title",
        &title,
        "--notes-file",
        &notes,
    ];
    argv.extend(paths.iter().map(String::as_str));
    let out = shell.run("gh", &argv, repo);
    ensure!(
        out.ok(),
        "creating the release failed: {}",
        out.stderr.trim()
    );
    Ok(())
}

/// The remote digest of one asset: GitHub's `sha256:` digest when present, else a download.
fn remote_digest(
    shell: &dyn Shell,
    repo: &Path,
    tag: &str,
    release: &Value,
    name: &str,
) -> anyhow::Result<Option<String>> {
    let Some(asset) = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"] == name)
    else {
        return Ok(None);
    };
    if let Some(digest) = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        return Ok(Some(digest.to_owned()));
    }
    let scratch = std::env::temp_dir().join(format!(
        "stutter-release-dl-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch)?;
    let out = shell.run(
        "gh",
        &[
            "release",
            "download",
            tag,
            "--pattern",
            name,
            "--dir",
            &scratch.to_string_lossy(),
        ],
        repo,
    );
    let digest = if out.ok() {
        file_sha256(&scratch.join(name)).ok()
    } else {
        None
    };
    let _ = fs::remove_dir_all(&scratch);
    ensure!(digest.is_some(), "cannot download asset {name} to check it");
    Ok(digest)
}

/// An existing release: the tag must match, present assets must be identical, missing ones are
/// uploaded. A differing asset is a hard failure.
fn complete(
    shell: &dyn Shell,
    repo: &Path,
    dir: &Path,
    tag: &str,
    assets: &[String],
    local: &Digests,
    release: &Value,
) -> anyhow::Result<()> {
    ensure!(
        release["tagName"] == tag,
        "the existing release does not target {tag}"
    );
    ensure!(
        release["isDraft"] != true,
        "the existing release is still a draft; publish or delete it first"
    );
    let mut missing = Vec::new();
    for name in assets {
        match remote_digest(shell, repo, tag, release, name)? {
            None => missing.push(dir.join(name).to_string_lossy().into_owned()),
            Some(remote) => ensure!(
                local.get(name) == Some(&remote),
                "asset {name} exists with a different digest; it is never overwritten"
            ),
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut argv = vec!["release", "upload", tag];
    argv.extend(missing.iter().map(String::as_str));
    let out = shell.run("gh", &argv, repo);
    ensure!(out.ok(), "uploading assets failed: {}", out.stderr.trim());
    Ok(())
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
