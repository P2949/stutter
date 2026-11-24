use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};

use super::*;
use crate::autotune::research::{
    crosswalk_evidence::{observations_from, verified},
    newest_record::{Newest, newest},
};

const CAMPAIGN: &str = "abababababababababababababababababababababababababababababababab";

struct Campaign {
    dir: tempfile::TempDir,
    file: File,
    uid: u32,
}

fn campaign() -> Campaign {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = File::open(dir.path()).unwrap();
    Campaign {
        dir,
        file,
        uid: crate::syscall::geteuid(),
    }
}

fn child(campaign: &Campaign, name: &str, rows: Vec<Value>) {
    let dir = campaign.dir.path().join("studies").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for path in [campaign.dir.path().join("studies"), dir.clone()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let file = dir.join(WINDOWS_FILE);
    std::fs::write(&file, serde_json::to_vec(&rows).unwrap()).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn window(psi: u64, gpu: u64, freq: u64) -> Value {
    json!({ "started_unix": 1_000, "ended_unix": 1_100, "psi_samples": psi, "gpu_samples": gpu, "cpu_freq_samples": freq })
}

fn observed(campaign: &Campaign) -> Option<Vec<String>> {
    let Newest::Found(record) =
        newest::<Value>(campaign.dir.path(), "crosswalk-observations", |_| 0)
    else {
        return None;
    };
    // What the report does with it: every row's evidence reopens and verifies.
    let checked = verified(Some(&record), &campaign.file, campaign.uid, CAMPAIGN)
        .expect("the evidence reopens");
    Some(
        observations_from(Some(&checked))
            .unwrap()
            .into_keys()
            .collect(),
    )
}

#[test]
fn only_the_dossiers_every_attempt_really_received_are_observed_and_their_evidence_reopens() {
    let c = campaign();
    child(
        &c,
        "0123456789abcdef",
        vec![window(50, 50, 50), window(50, 50, 50)],
    );
    child(&c, "fedcba9876543210", vec![window(50, 50, 50)]);
    produce(&c.file, c.uid, CAMPAIGN).unwrap();
    assert_eq!(
        observed(&c),
        Some(vec!["5.13".into(), "5.17".into(), "5.22".into()])
    );
    // Producing again changes nothing (write-once, the same content-named artifacts).
    produce(&c.file, c.uid, CAMPAIGN).unwrap();
    assert_eq!(observed(&c).unwrap().len(), 3);
}

#[test]
fn a_single_attempt_short_of_the_floor_or_a_campaign_with_no_windows_observes_nothing_for_that_dossier()
 {
    let c = campaign();
    // One window never saw CPU frequency events: that dossier is not observed; the others are.
    child(
        &c,
        "0123456789abcdef",
        vec![window(50, 50, 50), window(50, 50, 9)],
    );
    produce(&c.file, c.uid, CAMPAIGN).unwrap();
    assert_eq!(observed(&c), Some(vec!["5.13".into(), "5.22".into()]));
    // No windows at all: no record, not an empty one.
    let none = campaign();
    produce(&none.file, none.uid, CAMPAIGN).unwrap();
    assert_eq!(observed(&none), None);
    // Nothing reached the floor: no record.
    let thin = campaign();
    child(&thin, "0123456789abcdef", vec![window(1, 1, 1)]);
    produce(&thin.file, thin.uid, CAMPAIGN).unwrap();
    assert_eq!(observed(&thin), None);
    // Evidence for another campaign does not verify: the artifacts name the campaign they were produced for.
    let record = match newest::<Value>(c.dir.path(), "crosswalk-observations", |_| 0) {
        Newest::Found(record) => record,
        other => panic!("{other:?}"),
    };
    assert!(verified(Some(&record), &c.file, c.uid, &"cd".repeat(32)).is_err());
}

/// In a real campaign the study directory holds one subdirectory per campaign, and the producer writes the record
/// there. The report must read it from that same directory: looking in the study directory above it finds nothing.
#[test]
fn the_report_reads_the_observation_record_from_the_campaign_directory_the_producer_wrote_it_in() {
    let study = tempfile::tempdir().unwrap();
    std::fs::set_permissions(study.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let sub = study.path().join(&CAMPAIGN[..16]);
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o700)).unwrap();
    let inside = Campaign {
        dir: tempfile::tempdir().unwrap(),
        file: File::open(&sub).unwrap(),
        uid: crate::syscall::geteuid(),
    };
    // Children's windows live under the campaign directory, as in production.
    let children = sub.join("studies").join("0123456789abcdef");
    std::fs::create_dir_all(&children).unwrap();
    for path in [sub.join("studies"), children.clone()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let windows = children.join(WINDOWS_FILE);
    std::fs::write(
        &windows,
        serde_json::to_vec(&vec![window(50, 50, 50), window(50, 50, 50)]).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&windows, std::fs::Permissions::from_mode(0o600)).unwrap();
    produce(&inside.file, inside.uid, CAMPAIGN).unwrap();
    // The study directory above holds no record: the earlier lookup there could never find it.
    assert!(matches!(
        newest::<Value>(study.path(), "crosswalk-observations", |_| 0),
        Newest::None
    ));
    let record = crate::autotune::research::crosswalk_evidence::observed_in(
        &inside.file,
        inside.uid,
        CAMPAIGN,
    );
    let rows = record["rows"].as_array().expect("the record is found");
    assert_eq!(rows.len(), 3);
}

/// A child whose window file is present but cannot be read whole is a gap in the coverage, never silently skipped; and
/// the evidence names the raw window file of every child it read, so a changed window breaks the observation.
#[test]
fn an_unreadable_child_window_file_is_an_error_and_the_evidence_names_the_files_it_read() {
    let c = campaign();
    child(&c, "0123456789abcdef", vec![window(50, 50, 50)]);
    // A second child whose window file is not a list of windows.
    let broken = c.dir.path().join("studies").join("fedcba9876543210");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = broken.join(WINDOWS_FILE);
    std::fs::write(&file, b"not windows").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(produce(&c.file, c.uid, CAMPAIGN).is_err());
    assert_eq!(
        observed(&c),
        None,
        "nothing is observed from a partial read"
    );
    // With the broken child gone the evidence is produced and names the one raw file it read.
    std::fs::remove_dir_all(&broken).unwrap();
    produce(&c.file, c.uid, CAMPAIGN).unwrap();
    let record = match newest::<Value>(c.dir.path(), "crosswalk-observations", |_| 0) {
        Newest::Found(record) => record,
        other => panic!("{other:?}"),
    };
    let digest = record["rows"][0]["evidence_sha256"].as_str().unwrap();
    let artifact: Value = serde_json::from_slice(
        &std::fs::read(
            c.dir
                .path()
                .join(format!("crosswalk-evidence-{digest}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(artifact["sources"][0]["child"], "0123456789abcdef");
    assert_eq!(artifact["sources"].as_array().unwrap().len(), 1);
}
