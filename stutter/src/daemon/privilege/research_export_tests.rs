use super::*;

fn input(event: Event, transaction: &str) -> Input<'_> {
    Input {
        event,
        transaction,
        boot_id: "b",
        action_id: "cpu-affinity-profile:game",
        operation: "CpuAffinityProfile".into(),
        process: Some((4242, 9)),
        success: true,
    }
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Every record chains to the one before it by the hash of its own body, from the genesis value.
fn chain_is_intact(records: &[Value]) -> bool {
    let mut prev = GENESIS.to_owned();
    records.iter().enumerate().all(|(index, record)| {
        let mut body = record.clone();
        let hash = body.as_object_mut().unwrap().remove("hash").unwrap();
        let intact = record["prev"] == prev
            && record["seq"] == (index as u64 + 1)
            && hash == record_hash(&body);
        prev = hash.as_str().unwrap().to_owned();
        intact
    })
}

#[test]
fn a_transactions_lifecycle_is_exported_as_one_hash_chain_naming_the_exact_transaction_and_process()
{
    let dir = tempfile::tempdir().unwrap();
    let export = ResearchExport::in_dir(dir.path());
    for event in [Event::Prepared, Event::Applied, Event::Restored] {
        export.record(input(event, &"ab".repeat(32)));
    }
    let records = lines(&dir.path().join(FILE));
    assert_eq!(records.len(), 3);
    assert!(chain_is_intact(&records));
    assert_eq!(
        records
            .iter()
            .map(|r| r["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["prepared", "applied", "restored"]
    );
    assert!(records.iter().all(|r| r["transaction"] == "ab".repeat(32)));
    assert_eq!(
        records[0]["process"],
        json!({ "pid": 4242, "start_ticks": 9 })
    );
    // The controller can read the file but never write it.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir.path().join(FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o022, 0, "no group or other write");
    assert_eq!(mode & 0o044, 0o044, "readable by the controller");
}

#[test]
fn an_altered_or_dropped_record_breaks_the_chain() {
    let dir = tempfile::tempdir().unwrap();
    let export = ResearchExport::in_dir(dir.path());
    for event in [Event::Prepared, Event::Applied, Event::Restored] {
        export.record(input(event, "t1"));
    }
    let intact = lines(&dir.path().join(FILE));
    assert!(chain_is_intact(&intact));
    // A record changed in place.
    let mut altered = intact.clone();
    altered[1]["success"] = json!(false);
    assert!(!chain_is_intact(&altered));
    // A record removed from the middle.
    let mut dropped = intact.clone();
    dropped.remove(1);
    assert!(!chain_is_intact(&dropped));
    // Two records swapped.
    let mut swapped = intact;
    swapped.swap(1, 2);
    assert!(!chain_is_intact(&swapped));
}

#[test]
fn a_restarted_worker_continues_the_chain_and_a_torn_file_says_it_restarted() {
    let dir = tempfile::tempdir().unwrap();
    ResearchExport::in_dir(dir.path()).record(input(Event::Prepared, "t1"));
    // A new worker process picks the chain up where it ended.
    ResearchExport::in_dir(dir.path()).record(input(Event::Applied, "t1"));
    let records = lines(&dir.path().join(FILE));
    assert!(chain_is_intact(&records));
    // A last line that is not a record is never continued silently.
    let path = dir.path().join(FILE);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"seq\":3,\"ha")
        .unwrap();
    ResearchExport::in_dir(dir.path()).record(input(Event::Restored, "t1"));
    let text = std::fs::read_to_string(&path).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["chain_restarted"], true);
    assert_eq!(last["prev"], GENESIS);
}

#[test]
fn a_failed_write_never_fails_the_worker_and_the_gap_is_carried_on_the_next_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone");
    let export = ResearchExport::in_dir(&path);
    // The runtime directory does not exist: the write fails, silently for the caller.
    export.record(input(Event::Prepared, "t1"));
    export.record(input(Event::Applied, "t1"));
    std::fs::create_dir(&path).unwrap();
    export.record(input(Event::Restored, "t1"));
    let records = lines(&path.join(FILE));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["dropped_before"], 2);
    // And a worker with no export directory records nothing at all.
    ResearchExport::disabled().record(input(Event::Prepared, "t1"));
}
