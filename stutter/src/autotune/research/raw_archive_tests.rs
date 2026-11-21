use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{
    cyberpunk::{NativeBenchmarkFile, NativeBenchmarkMetrics},
    report::AttemptEvidenceV1,
};

struct World {
    _root: tempfile::TempDir,
    results: PathBuf,
    study: File,
    evidence: BTreeMap<u32, AttemptEvidenceV1>,
}

fn world() -> World {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let results = root.path().join("results");
    let run = results.join("run-0001");
    std::fs::create_dir_all(&run).unwrap();
    // A frame file bigger than an ordinary study record.
    let frames = vec![b'x'; 200 * 1024];
    std::fs::write(run.join("frames.csv"), &frames).unwrap();
    std::fs::write(run.join("summary.json"), b"{}").unwrap();
    let study_path = root.path().join("study");
    std::fs::create_dir(&study_path).unwrap();
    std::fs::set_permissions(&study_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = |name: &str, bytes: &[u8]| NativeBenchmarkFile {
        path: format!("run-0001/{name}"),
        size_bytes: bytes.len() as u64,
        sha256: stutter_core::hash::sha256_hex(bytes),
    };
    let mut item = crate::autotune::research::metric_plan::tests::item(1, None, 60.0);
    let native = item.native_completion.as_mut().unwrap();
    native.proton_source = ProtonSource::Ge;
    native.metrics = NativeBenchmarkMetrics {
        average_fps: 60.0,
        minimum_fps: None,
        maximum_fps: None,
    };
    native.files = vec![file("frames.csv", &frames), file("summary.json", b"{}")];
    World {
        study: File::open(&study_path).unwrap(),
        evidence: [(1, item)].into(),
        results,
        _root: root,
    }
}

#[test]
fn the_sealed_files_are_copied_hashed_privately_and_idempotently() {
    let w = world();
    let results = w.results.clone();
    archive(&w.study, &w.evidence, &move |_| results.clone()).unwrap();
    assert!(exists(&w.study));
    let copy = arm_root(&w.study, ProtonSource::Ge).join("run-0001/frames.csv");
    assert_eq!(std::fs::read(&copy).unwrap().len(), 200 * 1024);
    assert_eq!(
        std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // A second pass over the same files is a no-op, and the archive stands alone once the source goes.
    let results = w.results.clone();
    archive(&w.study, &w.evidence, &move |_| results.clone()).unwrap();
    std::fs::remove_dir_all(&w.results).unwrap();
    assert!(copy.is_file());
}

#[test]
fn a_file_that_differs_from_its_seal_or_from_an_existing_copy_is_refused() {
    let w = world();
    std::fs::write(w.results.join("run-0001/summary.json"), b"{ }").unwrap();
    let results = w.results.clone();
    assert!(archive(&w.study, &w.evidence, &move |_| results.clone()).is_err());

    let w = world();
    let results = w.results.clone();
    archive(&w.study, &w.evidence, &{
        let results = results.clone();
        move |_| results.clone()
    })
    .unwrap();
    // An archive copy edited afterwards is never silently accepted or overwritten.
    let copy = arm_root(&w.study, ProtonSource::Ge).join("run-0001/summary.json");
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&copy, b"{ }").unwrap();
    assert!(archive(&w.study, &w.evidence, &move |_| results.clone()).is_err());
}

#[test]
fn a_path_that_escapes_its_folder_or_an_oversized_file_is_refused() {
    for bad in ["../x", "/etc/passwd", "a/../../x", ""] {
        assert!(safe_relative(bad).is_none(), "{bad}");
    }
    assert!(safe_relative("run-0001/frames.csv").is_some());
    let mut w = world();
    w.evidence
        .get_mut(&1)
        .unwrap()
        .native_completion
        .as_mut()
        .unwrap()
        .files[0]
        .path = "../escape".into();
    let results = w.results.clone();
    assert!(archive(&w.study, &w.evidence, &move |_| results.clone()).is_err());
}
