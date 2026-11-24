use std::os::unix::fs::PermissionsExt;

use super::*;

const CAMPAIGN: &str = "abababababababababababababababababababababababababababababababab";

struct Study {
    dir: tempfile::TempDir,
    file: std::fs::File,
    uid: u32,
}

/// The raw window file of the one child the fixtures' evidence stands on.
const WINDOWS: &[u8] = b"[{\"attempt\":0}]";
const CHILD: &str = "0123456789abcdef";

fn study() -> Study {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let child = dir.path().join("studies").join(CHILD);
    std::fs::create_dir_all(&child).unwrap();
    for path in [dir.path().join("studies"), child.clone()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let windows = child.join(crate::autotune::research::resident_observer::WINDOWS_FILE);
    std::fs::write(&windows, WINDOWS).unwrap();
    std::fs::set_permissions(&windows, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = std::fs::File::open(dir.path()).unwrap();
    Study {
        dir,
        file,
        uid: crate::syscall::geteuid(),
    }
}

fn artifact(dossier: &str) -> Value {
    json!({
        "schema": 1, "dossier": dossier, "campaign_sha256": CAMPAIGN, "provider": "psi",
        "source_quality": "direct", "start_unix": 100, "end_unix": 200,
        "sources": [{ "child": CHILD, "sha256": stutter_core::hash::sha256_hex(WINDOWS) }],
    })
}

/// Stores the artifact beside the study under the name its own digest gives, and returns that digest.
fn store(study: &Study, value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap();
    let digest = stutter_core::hash::sha256_hex(&bytes);
    let path = study.dir.path().join(file_name(&digest));
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    digest
}

fn record(id: &str, digest: &str) -> Value {
    json!({ "rows": [{ "id": id, "evidence_sha256": digest }] })
}

fn check(study: &Study, value: &Value) -> Result<Value, &'static str> {
    verified(Some(value), &study.file, study.uid, CAMPAIGN)
}

#[test]
fn an_observation_stands_only_when_its_evidence_reopens_and_matches_the_campaign_and_dossier() {
    let study = study();
    let digest = store(&study, &artifact("5.1"));
    let ok = check(&study, &record("5.1", &digest)).unwrap();
    assert_eq!(ok["rows"][0]["provider"], "psi");
    assert_eq!(
        observations_from(Some(&ok)).unwrap().get("5.1"),
        Some(&digest)
    );
    // No record at all observes nothing, and that is not an error.
    assert_eq!(
        verified(None, &study.file, study.uid, CAMPAIGN),
        Ok(Value::Null)
    );
    assert!(observations_from(None).unwrap().is_empty());
    // A digest nothing on disk has, an artifact for another dossier, another campaign, or with a bad interval,
    // provider or quality.
    assert_eq!(
        check(&study, &record("5.1", &"cd".repeat(32))),
        Err(INVALID)
    );
    let other = store(&study, &artifact("5.2"));
    assert_eq!(check(&study, &record("5.1", &other)), Err(INVALID));
    for edit in [
        (|v: &mut Value| v["campaign_sha256"] = "cd".repeat(32).into()) as fn(&mut Value),
        |v| v["start_unix"] = 300.into(),
        |v| v["provider"] = "".into(),
        |v| v["source_quality"] = "guessed".into(),
        |v| v["schema"] = 2.into(),
        |v| v["extra"] = 1.into(),
        // No source, a source that is not a study directory, a digest that is not the file's, or a child that is not there.
        |v| v["sources"] = json!([]),
        |v| v["sources"][0]["child"] = "../etc".into(),
        |v| v["sources"][0]["sha256"] = "cd".repeat(32).into(),
        |v| v["sources"][0]["child"] = "fedcba9876543210".into(),
    ] {
        let mut bad = artifact("5.1");
        edit(&mut bad);
        let digest = store(&study, &bad);
        assert_eq!(
            check(&study, &record("5.1", &digest)),
            Err(INVALID),
            "{bad}"
        );
    }
    // A child's raw windows altered after the observation was written no longer hash to what the evidence names.
    let digest = store(&study, &artifact("5.1"));
    assert!(check(&study, &record("5.1", &digest)).is_ok());
    let windows = study
        .dir
        .path()
        .join("studies")
        .join(CHILD)
        .join(crate::autotune::research::resident_observer::WINDOWS_FILE);
    std::fs::write(&windows, b"[{\"attempt\":0,\"gpu_samples\":9999}]").unwrap();
    assert_eq!(check(&study, &record("5.1", &digest)), Err(INVALID));
    std::fs::write(&windows, WINDOWS).unwrap();
    // An artifact altered after the digest was named no longer hashes to it.
    let digest = store(&study, &artifact("5.1"));
    let path = study.dir.path().join(file_name(&digest));
    std::fs::write(&path, serde_json::to_vec(&artifact("5.3")).unwrap()).unwrap();
    assert_eq!(check(&study, &record("5.1", &digest)), Err(INVALID));
}

#[test]
fn a_malformed_or_repeated_record_is_an_integrity_failure_not_zero_observations() {
    let study = study();
    let digest = store(&study, &artifact("5.1"));
    for bad in [
        json!({ "rows": [{ "id": "9.9", "evidence_sha256": digest }] }),
        json!({ "rows": [{ "id": "5.1", "evidence_sha256": "xyz" }] }),
        json!({ "rows": [{ "id": "5.1" }] }),
        json!({ "rows": [{ "id": "5.1", "evidence_sha256": digest }, { "id": "5.1", "evidence_sha256": digest }] }),
        json!({ "rows": "none" }),
        json!("text"),
    ] {
        assert_eq!(check(&study, &bad), Err(INVALID), "{bad}");
        assert_eq!(observations_from(Some(&bad)), Err(INVALID), "{bad}");
    }
    assert_eq!(
        observations_from(Some(&json!({ "error": INVALID }))),
        Err(INVALID)
    );
    // An artifact that is group-readable is not a trusted campaign record.
    let path = study.dir.path().join(file_name(&digest));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(check(&study, &record("5.1", &digest)), Err(INVALID));
}
