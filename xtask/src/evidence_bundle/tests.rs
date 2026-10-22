use super::*;

pub(crate) struct Dir(pub(crate) std::path::PathBuf);
static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Dir {
    pub(crate) fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "stutter-evidence-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub(crate) fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const DIGEST: &str = "89abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567";

pub(crate) fn environment(suite: &str) -> Value {
    json!({"schema_version":1,"suite":suite,"evidence_kind":"controlled_real",
        "capture_id":format!("{suite}-0123456-1000"),"captured_unix_ms":1000,"commit":COMMIT,
        "source_dirty":false,"source_matches_binary":true,"source_matches_harness":true,
        "binary_version":"stutter 0.1.0 (git 0123456)","binary_sha256":DIGEST,"harness_sha256":DIGEST,
        "cargo_lock_sha256":DIGEST,"kernel_release":"6.18.55","distribution":"ID=testos",
        "cpu_model":"Test CPU","cpu_online":"0-3","rust":"rustc 1.0",
        "proc_version":"Linux version 6.18.55 (builder@buildhost)",
        "artifact_schema":{"evidence_result":1},"gpu_relevant":false})
}

/// A raw suite directory whose logs deliberately contain local identity.
pub(crate) fn raw(suite: &str, leak: &str) -> Dir {
    let raw = Dir::new("raw");
    fs::write(
        raw.join("environment.json"),
        serde_json::to_vec_pretty(&environment(suite)).unwrap(),
    )
    .unwrap();
    fs::write(
        raw.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"suite":suite,"evidence_kind":"controlled_real",
            "status":"PASS","capture_id":format!("{suite}-0123456-1000"),"captured_unix_ms":1000,
            "finished_unix_ms":2000,"scope":"unit test","error":null,
            "cases":[{"checkpoint":"one","status":"PASS"}],"not_claimed":["everything else"]}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::create_dir(raw.join("one")).unwrap();
    fs::write(
        raw.join("one/controller.stderr"),
        format!("started {leak}\n"),
    )
    .unwrap();
    fs::write(raw.join("one/before.json"), "[{\"mask\":\"0-3\"}]").unwrap();
    raw
}

fn local_leak() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/tester".to_owned());
    format!("home={home}/.local/state cwd=/tmp/scratch-xyz")
}

pub(crate) fn export(raw: &Dir, out: &Dir) -> anyhow::Result<std::path::PathBuf> {
    let bundle = out.join("bundle");
    export_bundle(Path::new("/nonexistent/checkout"), &raw.0, &bundle)?;
    Ok(bundle)
}

pub(crate) fn rewrite_manifest(bundle: &Path) {
    let mut manifest = String::new();
    for relative in verify::list_files(bundle).unwrap() {
        if relative != "MANIFEST.sha256" {
            manifest.push_str(&manifest_line(
                &sha256::sha256_hex(&fs::read(bundle.join(&relative)).unwrap()),
                &relative,
            ));
        }
    }
    fs::write(bundle.join("MANIFEST.sha256"), manifest).unwrap();
}

#[test]
fn export_sanitizes_and_the_independent_verifier_accepts_every_suite() {
    for suite in KNOWN_SUITES {
        let (raw, out) = (raw(suite, &local_leak()), Dir::new("out"));
        let bundle = export(&raw, &out).unwrap();
        verify_bundle(&bundle).unwrap();
        let log = fs::read_to_string(bundle.join("logs/one/controller.stderr")).unwrap();
        assert!(log.contains("<TMP>/scratch-xyz"), "{log}");
        assert!(!log.contains("/tmp/"));
        let environment = read_json(&bundle.join("environment.json")).unwrap();
        assert!(environment.get("proc_version").is_none());
        assert_eq!(environment["distribution"], "ID=testos");
        assert!(bundle.join("artifacts/result.json").is_file());
        assert!(bundle.join("README.md").is_file());
    }
}

#[test]
fn verifier_rejects_tampering_missing_files_and_stale_manifests() {
    let (raw, out) = (raw("live_recovery", "ok"), Dir::new("out"));
    let bundle = export(&raw, &out).unwrap();
    verify_bundle(&bundle).unwrap();
    fs::write(bundle.join("artifacts/one/before.json"), "[]").unwrap();
    assert!(verify_bundle(&bundle).is_err(), "digest mismatch");
    rewrite_manifest(&bundle);
    verify_bundle(&bundle).unwrap();
    fs::remove_file(bundle.join("validation.json")).unwrap();
    assert!(verify_bundle(&bundle).is_err(), "required file");
}

#[test]
fn verifier_is_fail_closed_on_privacy_even_with_a_valid_manifest() {
    for leak in [
        "path /home/someone/project",
        "scratch /tmp/x",
        "mail a.b@example.org",
        "password=hunter2",
        "SSH_AUTH_SOCK=/run/user/1000/ssh",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
    ] {
        let (raw, out) = (raw("live_recovery", "ok"), Dir::new("out"));
        let bundle = export(&raw, &out).unwrap();
        fs::write(bundle.join("logs/one/controller.stderr"), leak).unwrap();
        rewrite_manifest(&bundle);
        let error = format!("{:#}", verify_bundle(&bundle).unwrap_err());
        assert!(error.contains("privacy_scan"), "{leak}: {error}");
    }
}

#[test]
fn verifier_rejects_inconsistent_receipts() {
    let mutate = |change: &dyn Fn(&mut Value)| {
        let (raw, out) = (raw("live_recovery", "ok"), Dir::new("out"));
        let bundle = export(&raw, &out).unwrap();
        let mut receipt = read_json(&bundle.join("receipt.json")).unwrap();
        change(&mut receipt);
        fs::write(
            bundle.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        rewrite_manifest(&bundle);
        verify_bundle(&bundle).is_err()
    };
    assert!(!mutate(&|_| {}), "control must verify");
    assert!(mutate(&|r| r["suite"] = json!("live_affinity")), "suite");
    assert!(
        mutate(&|r| r["evidence_kind"] = json!("real_workload")),
        "kind"
    );
    assert!(
        mutate(&|r| r["evidence_kind"] = json!("guess")),
        "unknown kind"
    );
    assert!(mutate(&|r| r["status"] = json!("MAYBE")), "status");
    assert!(
        mutate(&|r| r["source"]["dirty"] = json!(true)),
        "dirty PASS"
    );
    assert!(mutate(&|r| r["not_claimed"] = json!([])), "limitations");
    assert!(
        mutate(&|r| r["environment"] = json!("/etc/passwd")),
        "absolute reference"
    );
    assert!(
        mutate(&|r| r["environment"] = json!("../environment.json")),
        "escaping reference"
    );
    assert!(
        mutate(&|r| r["cases"][0]["status"] = json!("FAIL")),
        "PASS with failing case"
    );
    assert!(mutate(&|r| r["source"]["commit"] = json!("zz")), "commit");
}

#[test]
fn verifier_rejects_unexpected_and_raw_artifacts() {
    for (name, bytes) in [
        ("artifacts/blob.bin", b"text".to_vec()),
        ("artifacts/id_rsa", b"text".to_vec()),
        ("artifacts/raw.dat", vec![0, 159, 146, 150]),
        ("stray.txt", b"text".to_vec()),
        ("other/file.json", b"{}".to_vec()),
    ] {
        let (raw, out) = (raw("live_recovery", "ok"), Dir::new("out"));
        let bundle = export(&raw, &out).unwrap();
        fs::create_dir_all(bundle.join(name).parent().unwrap()).unwrap();
        fs::write(bundle.join(name), bytes).unwrap();
        rewrite_manifest(&bundle);
        assert!(verify_bundle(&bundle).is_err(), "{name}");
    }
    let (raw, out) = (raw("live_recovery", "ok"), Dir::new("out"));
    let bundle = export(&raw, &out).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", bundle.join("artifacts/link")).unwrap();
    assert!(verify_bundle(&bundle).is_err(), "symlink");
}

#[test]
fn export_refuses_incomplete_provenance_and_unsafe_inputs_without_publishing() {
    // Historical evidence without capture identity must be rerun, not repaired.
    let (raw_dir, out) = (raw("live_recovery", "ok"), Dir::new("out"));
    let mut environment = environment("live_recovery");
    environment.as_object_mut().unwrap().remove("capture_id");
    fs::write(
        raw_dir.join("environment.json"),
        serde_json::to_vec(&environment).unwrap(),
    )
    .unwrap();
    let error = format!("{:#}", export(&raw_dir, &out).unwrap_err());
    assert!(error.contains("rerun"), "{error}");
    assert!(!out.join("bundle").exists() && !out.join("bundle.partial").exists());

    for (name, bytes) in [("core", vec![0_u8, 1, 2]), ("secret.key", b"x".to_vec())] {
        let (raw_dir, out) = (raw("live_recovery", "ok"), Dir::new("out"));
        fs::write(raw_dir.join(name), bytes).unwrap();
        assert!(export(&raw_dir, &out).is_err(), "{name}");
        assert!(!out.join("bundle").exists() && !out.join("bundle.partial").exists());
    }
    // A credential that survives sanitization blocks publication.
    let (raw_dir, out) = (raw("live_recovery", "password=hunter2"), Dir::new("out"));
    assert!(export(&raw_dir, &out).is_err());
    assert!(!out.join("bundle").exists());
}

#[test]
fn evidence_kinds_round_trip_and_unknown_is_rejected() {
    for kind in EvidenceKind::ALL {
        assert_eq!(EvidenceKind::parse(kind.as_str()).unwrap(), kind);
    }
    assert!(EvidenceKind::parse("real").is_err());
}

#[test]
fn real_workload_overhead_bundles_export_and_verify_with_their_own_kind() {
    let (raw_dir, out) = (raw("live_overhead", "ok"), Dir::new("out"));
    for file in ["environment.json", "result.json"] {
        let path = raw_dir.join(file);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["evidence_kind"] = json!("real_workload");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    let bundle = export(&raw_dir, &out).unwrap();
    verify_bundle(&bundle).unwrap();
    let receipt = read_json(&bundle.join("receipt.json")).unwrap();
    assert_eq!(receipt["evidence_kind"], "real_workload");
    assert_eq!(receipt["suite"], "live_overhead");
    // The controlled and real-workload runs are told apart by kind, not by suite id.
    let entry = crate::evidence_bundle::verified_index_entry(&bundle).unwrap();
    assert_eq!(entry["evidence_kind"], "real_workload");
}
