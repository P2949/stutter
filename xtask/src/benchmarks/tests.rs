use std::fs;

use super::{hyperfine, *};

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-bench-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn manifest_for(file: &str, bytes: &[u8]) -> ArtifactManifest {
    ArtifactManifest {
        schema_version: 1,
        tool: "hyperfine".into(),
        tool_version: "1.19.0".into(),
        argv: vec![
            "hyperfine".into(),
            "--export-json".into(),
            file.into(),
            "make".into(),
        ],
        build_id: "a".repeat(64),
        role: Role::Baseline,
        phase: "measure".into(),
        unit: "s".into(),
        run_id: "run-0001".into(),
        window_start_unix: 1_000,
        window_end_unix: 1_060,
        population: 20,
        lost: 0,
        exit_code: 0,
        files: vec![ArtifactFile {
            path: file.into(),
            sha256: sha256_hex(bytes),
        }],
    }
}

fn write_manifest(dir: &std::path::Path, manifest: &ArtifactManifest) -> std::path::PathBuf {
    let path = dir.join("manifest.json");
    fs::write(&path, serde_json::to_string(manifest).unwrap()).unwrap();
    path
}

const HYPERFINE: &str =
    r#"{"results":[{"command":"make","mean":1.2,"times":[1.1,1.2,1.3],"exit_codes":[0,0,0]}]}"#;

#[test]
fn a_matching_artifact_verifies_and_never_authorizes_anything() {
    let dir = dir("ok");
    fs::write(dir.join("out.json"), HYPERFINE).unwrap();
    let manifest = manifest_for("out.json", HYPERFINE.as_bytes());
    assert!(!manifest.authorizes_mutation());
    let verified = verify(&write_manifest(&dir, &manifest)).unwrap();
    assert!(verified.clean_exit);
    assert_eq!(verified.loss_fraction, 0.0);
    assert_eq!(verified.files.len(), 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_tampered_truncated_missing_or_escaping_artifact_is_refused() {
    let dir = dir("bad");
    fs::write(dir.join("out.json"), HYPERFINE).unwrap();
    let manifest = manifest_for("out.json", HYPERFINE.as_bytes());
    let path = write_manifest(&dir, &manifest);
    fs::write(dir.join("out.json"), &HYPERFINE[..HYPERFINE.len() - 5]).unwrap();
    assert!(
        verify(&path)
            .unwrap_err()
            .to_string()
            .contains("altered or truncated")
    );
    fs::remove_file(dir.join("out.json")).unwrap();
    assert!(verify(&path).unwrap_err().to_string().contains("missing"));
    let escaping = ArtifactManifest {
        files: vec![ArtifactFile {
            path: "../etc/passwd".into(),
            sha256: "a".repeat(64),
        }],
        ..manifest.clone()
    };
    assert!(
        verify(&write_manifest(&dir, &escaping))
            .unwrap_err()
            .to_string()
            .contains("leaves")
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_oversize_file_and_a_symlinked_artifact_are_refused() {
    let dir = dir("big");
    let big = vec![b'x'; (MAX_ARTIFACT_BYTES + 1) as usize];
    fs::write(dir.join("big.json"), &big).unwrap();
    let manifest = manifest_for("big.json", &big);
    assert!(
        verify(&write_manifest(&dir, &manifest))
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    fs::write(dir.join("real.json"), HYPERFINE).unwrap();
    std::os::unix::fs::symlink(dir.join("real.json"), dir.join("link.json")).unwrap();
    let linked = manifest_for("link.json", HYPERFINE.as_bytes());
    assert!(
        verify(&write_manifest(&dir, &linked))
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_manifest_must_name_everything_that_identifies_a_run() {
    let base = manifest_for("out.json", HYPERFINE.as_bytes());
    assert!(base.validate().is_ok());
    let mut cases: Vec<ArtifactManifest> = Vec::new();
    for mutate in [
        |m: &mut ArtifactManifest| m.tool.clear(),
        |m: &mut ArtifactManifest| m.build_id.clear(),
        |m: &mut ArtifactManifest| m.run_id.clear(),
        |m: &mut ArtifactManifest| m.argv.clear(),
        |m: &mut ArtifactManifest| m.population = 0,
        |m: &mut ArtifactManifest| m.window_end_unix = 1,
        |m: &mut ArtifactManifest| m.files.clear(),
        |m: &mut ArtifactManifest| m.files[0].sha256 = "short".into(),
    ] {
        let mut changed = base.clone();
        mutate(&mut changed);
        cases.push(changed);
    }
    for case in cases {
        assert!(case.validate().is_err(), "{case:?}");
    }
}

#[test]
fn a_lossy_capture_and_a_failing_run_are_visible_not_hidden() {
    let dir = dir("loss");
    fs::write(dir.join("out.json"), HYPERFINE).unwrap();
    let mut manifest = manifest_for("out.json", HYPERFINE.as_bytes());
    manifest.lost = 20;
    manifest.exit_code = 2;
    let verified = verify(&write_manifest(&dir, &manifest)).unwrap();
    assert_eq!(verified.loss_fraction, 0.5);
    assert!(!verified.clean_exit);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hyperfine_exports_import_as_samples_and_damaged_ones_are_refused() {
    let runs = hyperfine::import(HYPERFINE.as_bytes()).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].seconds, vec![1.1, 1.2, 1.3]);
    assert!(runs[0].all_exited_zero);
    let failing = r#"{"results":[{"command":"x","times":[1.0],"exit_codes":[0,1]}]}"#;
    assert!(!hyperfine::import(failing.as_bytes()).unwrap()[0].all_exited_zero);
    for bad in [
        "not json",
        r#"{"results":[]}"#,
        r#"{"other":1}"#,
        r#"{"results":[{"command":"x"}]}"#,
        r#"{"results":[{"command":"x","times":[-1.0]}]}"#,
        r#"{"results":[{"command":"x","times":["1"]}]}"#,
        r#"{"results":[{"command":"x","times":[]}]}"#,
    ] {
        assert!(hyperfine::import(bad.as_bytes()).is_err(), "{bad}");
    }
}

/// Seeded pseudo-random bytes: failures reproduce.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.max(1);
    (0..len)
        .map(|_| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

#[test]
fn arbitrary_bytes_never_panic_the_importers_or_become_an_accepted_artifact() {
    for seed in 1..400 {
        let bytes = noise(seed, (seed as usize * 7) % 600);
        assert!(hyperfine::import(&bytes).is_err(), "seed {seed}");
        assert!(
            serde_json::from_slice::<ArtifactManifest>(&bytes).is_err(),
            "seed {seed}"
        );
    }
    // JSON-shaped but hostile: still refused or harmlessly empty, never a panic.
    for hostile in [
        r#"{"results":[{"times":[1e999]}]}"#,
        r#"{"results":[{"times":[0.1,null]}]}"#,
        r#"{"results":"x"}"#,
        r#"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[["#,
    ] {
        assert!(hyperfine::import(hostile.as_bytes()).is_err(), "{hostile}");
    }
}
