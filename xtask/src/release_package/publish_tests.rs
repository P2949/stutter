use std::{cell::RefCell, collections::BTreeMap};

use super::*;

const COMMIT: &str = "75d2f99d1e2f8c0bab50b747db33ead0061cb165";
const OTHER: &str = "fec0f0ec12ba85fa9e1744b880a4a3ec32bd10e8";

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A release directory with every asset and a valid `SHA256SUMS`, plus a notes file.
fn package(name: &str) -> (Dir, PublishArgs) {
    let root = std::env::temp_dir().join(format!("stutter-publish-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let mut sums = String::new();
    for asset in asset_names("0.1.0") {
        if asset == SUMS_NAME {
            continue;
        }
        let body = if asset == MANIFEST_NAME {
            format!(r#"{{"version":"0.1.0","git_revision":"{COMMIT}"}}"#)
        } else {
            format!("content of {asset}")
        };
        fs::write(root.join(&asset), body).unwrap();
        sums.push_str(&format!(
            "{}  {asset}\n",
            file_sha256(&root.join(&asset)).unwrap()
        ));
    }
    fs::write(root.join(SUMS_NAME), sums).unwrap();
    fs::write(root.join("notes.md"), "notes").unwrap();
    let args = PublishArgs {
        dir: root.clone(),
        acceptance_bundle: vec![root.join("bundle")],
        notes: root.join("notes.md"),
    };
    (Dir(root), args)
}

struct Github {
    local_tag: String,
    origin_tag: String,
    /// `None` = no release; otherwise asset name -> sha256 digest.
    release: RefCell<Option<BTreeMap<String, String>>>,
    calls: RefCell<Vec<String>>,
}

impl Github {
    fn new() -> Self {
        Self {
            local_tag: COMMIT.to_owned(),
            origin_tag: COMMIT.to_owned(),
            release: RefCell::new(None),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn mutations(&self) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.contains("release create") || c.contains("release upload"))
            .count()
    }
}

fn digests_of(paths: &[&str]) -> BTreeMap<String, String> {
    paths
        .iter()
        .filter(|p| {
            Path::new(p)
                .file_name()
                .is_some_and(|n| asset_names("0.1.0").contains(&n.to_string_lossy().into_owned()))
        })
        .map(|p| {
            (
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                file_sha256(Path::new(p)).unwrap(),
            )
        })
        .collect()
}

impl Shell for Github {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _cwd: &Path,
    ) -> crate::validation_campaign::shell::ShellOutput {
        use crate::validation_campaign::shell::ShellOutput;
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        let ok = |stdout: String| ShellOutput {
            code: 0,
            stdout,
            stderr: String::new(),
        };
        match (program, args) {
            ("git", ["rev-parse", ..]) => ok(format!("{}\n", self.local_tag)),
            ("git", ["ls-remote", ..]) => ok(format!(
                "1111111111111111111111111111111111111111\trefs/tags/v0.1.0\n{}\trefs/tags/v0.1.0^{{}}\n",
                self.origin_tag
            )),
            ("gh", ["release", "view", ..]) => match &*self.release.borrow() {
                None => ShellOutput {
                    code: 1,
                    stdout: String::new(),
                    stderr: "release not found".to_owned(),
                },
                Some(assets) => ok(serde_json::json!({
                    "tagName": "v0.1.0",
                    "isDraft": false,
                    "assets": assets.iter().map(|(name, digest)| serde_json::json!({
                        "name": name, "digest": format!("sha256:{digest}")
                    })).collect::<Vec<_>>(),
                })
                .to_string()),
            },
            ("gh", ["release", "create", rest @ ..]) => {
                *self.release.borrow_mut() = Some(digests_of(rest));
                ok(String::new())
            }
            ("gh", ["release", "upload", _, rest @ ..]) => {
                if let Some(assets) = self.release.borrow_mut().as_mut() {
                    assets.extend(digests_of(rest));
                }
                ok(String::new())
            }
            _ => ShellOutput {
                code: 1,
                stdout: String::new(),
                stderr: format!("unexpected command: {line}"),
            },
        }
    }

    fn sleep(&self, _duration: std::time::Duration) {}
}

fn pass(_: &VerifyArgs) -> anyhow::Result<()> {
    Ok(())
}

fn publish(github: &Github, dir: &Dir, args: &PublishArgs) -> anyhow::Result<()> {
    publish_with(github, &pass, Path::new("."), &dir.0, args)
}

#[test]
fn a_new_release_is_created_with_exactly_the_six_assets() {
    let (dir, args) = package("create");
    let github = Github::new();
    publish(&github, &dir, &args).unwrap();
    let release = github.release.borrow().clone().unwrap();
    assert_eq!(release.len(), 6);
    assert_eq!(github.mutations(), 1);
    let create = github
        .calls
        .borrow()
        .iter()
        .find(|c| c.contains("release create"))
        .cloned()
        .unwrap();
    assert!(create.contains("--verify-tag"), "{create}");
}

#[test]
fn an_identical_existing_release_is_a_no_op() {
    let (dir, args) = package("idempotent");
    let github = Github::new();
    publish(&github, &dir, &args).unwrap();
    let before = github.mutations();
    publish(&github, &dir, &args).unwrap();
    assert_eq!(github.mutations(), before);
}

#[test]
fn only_missing_assets_are_uploaded() {
    let (dir, args) = package("missing");
    let github = Github::new();
    publish(&github, &dir, &args).unwrap();
    github
        .release
        .borrow_mut()
        .as_mut()
        .unwrap()
        .remove(SBOM_NAME);
    publish(&github, &dir, &args).unwrap();
    let uploads: Vec<String> = github
        .calls
        .borrow()
        .iter()
        .filter(|c| c.contains("release upload"))
        .cloned()
        .collect();
    assert_eq!(uploads.len(), 1);
    assert!(uploads[0].contains(SBOM_NAME) && !uploads[0].contains(SUPPORT_NAME));
    assert_eq!(github.release.borrow().as_ref().unwrap().len(), 6);
}

#[test]
fn a_wrong_tag_is_refused_before_any_mutation() {
    let (dir, args) = package("tag");
    for (local, origin) in [(OTHER, COMMIT), (COMMIT, OTHER)] {
        let github = Github {
            local_tag: local.to_owned(),
            origin_tag: origin.to_owned(),
            ..Github::new()
        };
        let error = publish(&github, &dir, &args).unwrap_err().to_string();
        assert!(error.contains("does not resolve"), "{error}");
        assert_eq!(github.mutations(), 0);
    }
}

#[test]
fn a_differing_remote_asset_is_never_overwritten() {
    let (dir, args) = package("mismatch");
    let github = Github::new();
    publish(&github, &dir, &args).unwrap();
    github
        .release
        .borrow_mut()
        .as_mut()
        .unwrap()
        .insert(SUPPORT_NAME.to_owned(), "0".repeat(64));
    let before = github.mutations();
    let error = publish(&github, &dir, &args).unwrap_err().to_string();
    assert!(error.contains("different digest"), "{error}");
    assert_eq!(github.mutations(), before);
}

#[test]
fn acceptance_bundles_are_mandatory() {
    let (dir, mut args) = package("bundles");
    args.acceptance_bundle.clear();
    let github = Github::new();
    let error = publish(&github, &dir, &args).unwrap_err().to_string();
    assert!(error.contains("acceptance bundles"), "{error}");
    assert!(github.calls.borrow().is_empty());
}

#[test]
fn failed_package_or_bundle_verification_blocks_publication() {
    let (dir, args) = package("verify");
    let github = Github::new();
    for reason in ["bundle is for another commit", "digest mismatch"] {
        let verify = move |_: &VerifyArgs| Err(anyhow::anyhow!(reason));
        let error = publish_with(&github, &verify, Path::new("."), &dir.0, &args)
            .unwrap_err()
            .to_string();
        assert!(error.contains("nothing was published"), "{error}");
    }
    assert!(github.calls.borrow().is_empty());
}

#[test]
fn a_tampered_package_file_is_refused() {
    let (dir, args) = package("tamper");
    fs::write(dir.0.join(SBOM_NAME), "tampered").unwrap();
    let github = Github::new();
    let error = publish(&github, &dir, &args).unwrap_err().to_string();
    assert!(error.contains("SHA256SUMS"), "{error}");
    assert_eq!(github.mutations(), 0);
}
