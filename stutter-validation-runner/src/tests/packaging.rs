use std::{fs, path::Path};

fn repository() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn the_product_package_cannot_contain_the_validation_runner() {
    let root = repository();
    let manifest = fs::read_to_string(root.join("packaging/tarball/MANIFEST.txt")).unwrap();
    for line in manifest.lines() {
        assert!(
            !line.contains("validation"),
            "the v0.1.0 product package must not ship validation infrastructure: {line}"
        );
    }
    // Neither the release packager's service-unit mapping nor the tarball README lists it.
    let readme = fs::read_to_string(root.join("packaging/tarball/README.md")).unwrap();
    assert!(!readme.contains("validation-runner"));
}

#[test]
fn the_runner_and_its_units_are_validation_only() {
    let root = repository();
    let service =
        fs::read_to_string(root.join("packaging/systemd/stutter-validation-runner.service"))
            .unwrap();
    for required in [
        "Type=simple",
        "ExecStart=/usr/local/libexec/stutter-validation-runner serve",
        "RuntimeDirectory=stutter-validation",
        "StateDirectory=stutter-validation",
        "Restart=on-failure",
        "RestrictAddressFamilies=AF_UNIX",
        "PrivateTmp=true",
        "ProtectHome=",
    ] {
        assert!(service.contains(required), "runner unit lacks {required}");
    }
    let resume =
        fs::read_to_string(root.join("packaging/systemd/stutter-validation-resume.service"))
            .unwrap();
    for required in [
        "Type=oneshot",
        "After=local-fs.target",
        "Before=multi-user.target",
        "ExecStart=/usr/local/libexec/stutter-validation-runner resume --all",
    ] {
        assert!(resume.contains(required), "resume unit lacks {required}");
    }
    // The runner is a workspace member built by the ordinary gate, but is not a product crate.
    let workspace = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        workspace.matches("stutter-validation-runner").count() >= 2,
        "member and default member"
    );
}
