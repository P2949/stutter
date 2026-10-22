use std::os::unix::fs::PermissionsExt;

use super::*;

struct Repo(PathBuf);
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    run(root, "git", args).unwrap()
}

/// A minimal committed repository with the packaged layout and a stub binary whose
/// `--version` names the real HEAD, so package/verify exercise real git, tar and gzip.
fn repo(name: &str) -> (Repo, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("stutter-release-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for dir in [
        "packaging/tarball",
        "packaging/systemd",
        "packaging/openrc",
        "docs",
        "docs/evidence",
    ] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(
        root.join("Cargo.lock"),
        "[[package]]\nname = \"stutter\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nchecksum = \"abc\"\n",
    )
    .unwrap();
    fs::write(
        root.join("packaging/tarball/MANIFEST.txt"),
        "bin/stutter\nlib/stutter/stutter.bpf.o\nshare/doc/stutter/INSTALL.md\nshare/stutter/systemd/a.service\nshare/stutter/openrc/a\netc/stutter/\n",
    )
    .unwrap();
    fs::write(root.join("docs/INSTALL.md"), "install").unwrap();
    fs::write(root.join("packaging/systemd/a.service"), "[Unit]").unwrap();
    fs::write(root.join("packaging/openrc/a"), "#!/sbin/openrc-run").unwrap();
    fs::write(
        root.join("docs/evidence/index.json"),
        r#"{"schema_version":1,"entries":[]}"#,
    )
    .unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "t@example.org"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-qm", "release"],
    ] {
        git(&root, args);
    }
    let short = git(&root, &["rev-parse", "--short=7", "HEAD"]);
    let scratch =
        std::env::temp_dir().join(format!("stutter-release-bin-{name}-{}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();
    let binary = scratch.join("stutter");
    fs::write(
        &binary,
        format!("#!/bin/sh\necho 'stutter 0.1.0 (git {short})'\n"),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let object = scratch.join("stutter.bpf.o");
    let mut elf = b"\x7fELF".to_vec();
    elf.extend(vec![0_u8; 128]);
    fs::write(&object, elf).unwrap();
    (Repo(root), binary, object)
}

fn package(root: &Path, binary: &Path, object: &Path, out: &Path) -> anyhow::Result<()> {
    package_release(
        root,
        &PackageArgs {
            binary: binary.to_owned(),
            bpf_object: object.to_owned(),
            out: out.to_owned(),
        },
    )
}

fn verify(dir: &Path, require_acceptance: bool) -> anyhow::Result<()> {
    verify_release(&VerifyArgs {
        dir: dir.to_owned(),
        require_acceptance,
    })
}

#[test]
fn packaged_release_verifies_and_every_tamper_is_detected() {
    let (repo, binary, object) = repo("ok");
    let out = repo
        .0
        .join("../")
        .join(format!("stutter-release-out-{}", std::process::id()));
    let out = out.components().collect::<PathBuf>();
    let _ = fs::remove_dir_all(&out);
    package(&repo.0, &binary, &object, &out).unwrap();
    verify(&out, false).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(out.join(MANIFEST_NAME)).unwrap()).unwrap();
    assert_eq!(manifest["bpf_object"]["required_in_package"], true);
    assert_eq!(manifest["supported_capabilities"], json!([]));
    // Reproducible: the same commit yields byte-identical archives.
    let again = out.with_extension("again");
    let _ = fs::remove_dir_all(&again);
    package(&repo.0, &binary, &object, &again).unwrap();
    assert_eq!(
        fs::read(out.join("stutter-0.1.0-linux-x86_64.tar.gz")).unwrap(),
        fs::read(again.join("stutter-0.1.0-linux-x86_64.tar.gz")).unwrap()
    );
    let _ = fs::remove_dir_all(&again);
    for name in [
        "stutter-0.1.0-linux-x86_64.tar.gz",
        "stutter-0.1.0-source.tar.gz",
        "sbom.cdx.json",
        "SUPPORT.md",
        MANIFEST_NAME,
    ] {
        let path = out.join(name);
        let original = fs::read(&path).unwrap();
        let mut tampered = original.clone();
        tampered.push(b'x');
        fs::write(&path, tampered).unwrap();
        assert!(verify(&out, false).is_err(), "{name} tamper must fail");
        fs::write(&path, original).unwrap();
        verify(&out, false).unwrap();
    }
    // A forged support claim fails even with matching digests.
    let mut forged = manifest.clone();
    forged["supported_capabilities"] = json!(["cpu_affinity_experiment"]);
    fs::write(
        out.join(MANIFEST_NAME),
        serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .unwrap();
    assert!(verify(&out, false).is_err());
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn packaging_fails_closed_on_missing_object_wrong_binary_and_dirty_tree() {
    let (repo, binary, object) = repo("fail");
    let out = |n: &str| {
        std::env::temp_dir().join(format!("stutter-release-fail-{n}-{}", std::process::id()))
    };
    // Missing or non-ELF eBPF object is a release failure, never a warning.
    assert!(package(&repo.0, &binary, &repo.0.join("absent.o"), &out("missing")).is_err());
    let fake = repo.0.join("../fake.o");
    fs::write(&fake, b"not elf").unwrap();
    assert!(package(&repo.0, &binary, &fake, &out("elf")).is_err());
    // Binary built from another revision.
    let stale = binary.with_file_name("stale");
    fs::write(&stale, "#!/bin/sh\necho 'stutter 0.1.0 (git 0000000)'\n").unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(package(&repo.0, &stale, &object, &out("stale")).is_err());
    // Dirty tree.
    fs::write(repo.0.join("untracked"), "x").unwrap();
    assert!(package(&repo.0, &binary, &object, &out("dirty")).is_err());
    for n in ["missing", "elf", "stale", "dirty"] {
        assert!(!out(n).join("SHA256SUMS").exists());
        let _ = fs::remove_dir_all(out(n));
    }
}

#[test]
fn acceptance_for_the_exact_commit_is_required_when_asked() {
    let (repo, binary, object) = repo("accept");
    let out =
        std::env::temp_dir().join(format!("stutter-release-accept-out-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    package(&repo.0, &binary, &object, &out).unwrap();
    // Nothing is Supported, so a release claiming supported behavior has no basis.
    assert!(verify(&out, true).is_err());
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn binary_version_parsing_requires_a_git_revision() {
    assert_eq!(
        parse_binary_version("stutter 0.1.0 (git abc1234)").unwrap(),
        ("0.1.0".to_owned(), "abc1234".to_owned())
    );
    assert!(parse_binary_version("stutter 0.1.0").is_err());
    assert!(parse_binary_version("other 0.1.0 (git x)").is_err());
}
