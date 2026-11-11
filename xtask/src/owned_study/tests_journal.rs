//! E9: resume and analysis fail closed on any damaged, ambiguous or mismatched record.
use std::{cell::Cell, fs, os::unix::fs::PermissionsExt, path::Path};

use super::{
    host::HostState,
    journal::{self, Mode, Record, StudyReadError, begin_slot, commit_slot},
    manifest::{Manifest, Slot},
    run::{Completed, SlotRunner, run_with},
    tests::{manifest, record, schedule_records, scratch, write_legacy},
};

const CTX: &str = "ctx-1";

type Outcome = anyhow::Result<Result<Completed, String>>;

fn done(ns: u64) -> Outcome {
    Ok(Ok(Completed {
        elapsed_ns: ns,
        output_sha256: None,
    }))
}

struct Fake {
    prepared: Cell<usize>,
    ran: Cell<usize>,
    outcome: Box<dyn Fn(&Slot) -> Outcome>,
}

impl Fake {
    fn ok() -> Self {
        Self {
            prepared: Cell::new(0),
            ran: Cell::new(0),
            outcome: Box::new(|_| done(1_000)),
        }
    }
}

impl SlotRunner for Fake {
    fn prepare(&self, _slot: &Slot) -> anyhow::Result<()> {
        self.prepared.set(self.prepared.get() + 1);
        Ok(())
    }
    fn run_slot(&self, slot: &Slot) -> Outcome {
        self.ran.set(self.ran.get() + 1);
        (self.outcome)(slot)
    }
}

fn safe() -> HostState {
    HostState::Safe
}

fn fresh(name: &str) -> (std::path::PathBuf, Manifest) {
    let dir = scratch(name);
    let m = manifest(&dir);
    (dir, m)
}

fn committed(dir: &Path, m: &Manifest, count: usize) {
    let slots = dir.join("slots");
    fs::create_dir_all(&slots).expect("slots");
    for r in schedule_records(m, Some(CTX), |_, _| Some(7))
        .iter()
        .take(count)
    {
        begin_slot(&slots, &r.slot).expect("begin");
        commit_slot(&slots, r).expect("commit");
    }
}

fn resume(dir: &Path, m: &Manifest) -> Result<journal::Verified, StudyReadError> {
    journal::load(dir, m, &Mode::Resume(CTX.into()), true)
}

#[test]
fn a_full_run_commits_every_slot_and_an_idempotent_resume_launches_nothing() {
    let (dir, m) = fresh("j-run");
    let fake = Fake::ok();
    run_with(&m, &dir, CTX, &safe, &fake).expect("run");
    assert_eq!((fake.prepared.get(), fake.ran.get()), (30, 30));
    let verified = resume(&dir, &m).expect("verified");
    assert_eq!((verified.records.len(), verified.pending.len()), (30, 0));
    assert!(
        verified
            .records
            .iter()
            .all(|r| r.context.as_deref() == Some(CTX))
    );
    let again = Fake::ok();
    run_with(&m, &dir, CTX, &safe, &again).expect("resume");
    assert_eq!(
        (again.prepared.get(), again.ran.get()),
        (0, 0),
        "a finished study repeats nothing"
    );
}

#[test]
fn an_interrupted_study_resumes_exactly_where_it_stopped_with_no_duplicate_slot() {
    let (dir, m) = fresh("j-resume");
    committed(&dir, &m, 7);
    let fake = Fake::ok();
    run_with(&m, &dir, CTX, &safe, &fake).expect("resume");
    assert_eq!(fake.ran.get(), 23);
    let all = resume(&dir, &m).expect("all");
    let ids: std::collections::BTreeSet<_> = all.records.iter().map(|r| r.slot.clone()).collect();
    assert_eq!((all.records.len(), ids.len()), (30, 30));
}

#[test]
fn a_slot_started_but_never_committed_is_ambiguous_and_is_never_rerun() {
    let (dir, m) = fresh("j-ambiguous");
    committed(&dir, &m, 3);
    let id = m.schedule()[3].id.clone();
    begin_slot(&dir.join("slots"), &id).expect("begin only: the process died mid-slot");
    assert!(matches!(
        resume(&dir, &m),
        Err(StudyReadError::Ambiguous(_))
    ));
    let fake = Fake::ok();
    assert!(run_with(&m, &dir, CTX, &safe, &fake).is_err());
    assert_eq!(
        (fake.prepared.get(), fake.ran.get()),
        (0, 0),
        "zero launches on failed verification"
    );
}

#[test]
fn a_torn_commit_leaves_a_temporary_file_that_blocks_resume() {
    let (dir, m) = fresh("j-torn");
    committed(&dir, &m, 2);
    let id = m.schedule()[2].id.clone();
    fs::write(
        dir.join("slots").join(format!("{id}.json.tmp")),
        "{\"record\":",
    )
    .expect("tmp");
    assert!(matches!(
        resume(&dir, &m),
        Err(StudyReadError::Ambiguous(_))
    ));
}

#[test]
fn a_corrupted_or_edited_committed_record_is_detected() {
    let (dir, m) = fresh("j-corrupt");
    committed(&dir, &m, 3);
    let id = m.schedule()[1].id.clone();
    let path = dir.join("slots").join(format!("{id}.json"));
    let text = fs::read_to_string(&path)
        .expect("read")
        .replace("\"elapsed_ns\":7", "\"elapsed_ns\":8");
    fs::write(&path, text).expect("edit");
    assert!(matches!(
        resume(&dir, &m),
        Err(StudyReadError::Malformed(_))
    ));
    fs::write(&path, "not json").expect("garble");
    assert!(matches!(
        resume(&dir, &m),
        Err(StudyReadError::Malformed(_))
    ));
}

#[test]
fn unknown_entries_and_gaps_and_out_of_order_commits_are_refused() {
    let (dir, m) = fresh("j-unknown");
    committed(&dir, &m, 2);
    fs::write(dir.join("slots/b99-baseline.json"), "{}").expect("unknown");
    assert!(matches!(resume(&dir, &m), Err(StudyReadError::Unknown(_))));
    fs::remove_file(dir.join("slots/b99-baseline.json")).expect("rm");
    let later = schedule_records(&m, Some(CTX), |_, _| Some(7))[5].clone();
    commit_slot(&dir.join("slots"), &later)
        .expect("a later slot committed while earlier ones are missing");
    assert!(matches!(
        resume(&dir, &m),
        Err(StudyReadError::OutOfOrder(_))
    ));
}

#[test]
fn a_permission_denied_results_file_is_not_an_empty_study() {
    let (dir, m) = fresh("j-perm");
    let records = schedule_records(&m, Some(CTX), |_, _| Some(7));
    let path = write_legacy(&dir, &records).join("results.jsonl");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");
    if fs::read(&path).is_err() {
        assert!(matches!(
            journal::load(&dir, &m, &Mode::Analyze, false),
            Err(StudyReadError::Unreadable(..))
        ));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore");
    let fresh_dir = scratch("j-absent");
    assert!(
        journal::load(&fresh_dir, &m, &Mode::Analyze, false).is_err(),
        "analysis of a missing store is an error"
    );
    assert!(
        resume(&fresh_dir, &m)
            .expect("a brand-new study is fine")
            .pending
            .len()
            == 30
    );
}

#[test]
fn legacy_jsonl_damage_is_never_skipped() {
    let (dir, m) = fresh("j-legacy-bad");
    let records = schedule_records(&m, None, |_, _| Some(7));
    let line = |r: &Record| serde_json::to_string(r).expect("json");
    let good: Vec<String> = records.iter().take(4).map(line).collect();
    let cases: Vec<(&str, String)> = vec![
        (
            "malformed middle line",
            format!("{}\n{{broken\n{}\n{}\n", good[0], good[2], good[3]),
        ),
        (
            "truncated final line",
            format!("{}\n{}\n{}", good[0], good[1], &good[2][..20]),
        ),
        ("blank line", format!("{}\n\n{}\n", good[0], good[1])),
        (
            "duplicate slot",
            format!("{}\n{}\n{}\n", good[0], good[1], good[1]),
        ),
        (
            "unknown field",
            format!(
                "{}\n",
                good[0].replace("\"valid\"", "\"extra\":1,\"valid\"")
            ),
        ),
    ];
    for (what, text) in cases {
        fs::write(dir.join("results.jsonl"), text).expect("write");
        assert!(
            journal::load(&dir, &m, &Mode::Analyze, false).is_err(),
            "{what}"
        );
    }
}

#[test]
fn mismatched_study_manifest_arm_block_and_order_are_refused() {
    let (_, m) = fresh("j-mismatch");
    let base = schedule_records(&m, Some(CTX), |_, _| Some(7));
    let bad = |edit: &dyn Fn(&mut Vec<Record>)| {
        let mut r = base[..6].to_vec();
        edit(&mut r);
        journal::verify_records(&m, r, &Mode::Resume(CTX.into()))
    };
    assert!(matches!(
        bad(&|r| r[2].study_id = "other".into()),
        Err(StudyReadError::Mismatch(_))
    ));
    assert!(matches!(
        bad(&|r| r[2].manifest_sha256 = "x".into()),
        Err(StudyReadError::Mismatch(_))
    ));
    assert!(matches!(
        bad(&|r| r[2].arm = "split".into()),
        Err(StudyReadError::Mismatch(_))
    ));
    assert!(matches!(
        bad(&|r| r[2].block = 5),
        Err(StudyReadError::Mismatch(_))
    ));
    assert!(matches!(
        bad(&|r| r.swap(1, 2)),
        Err(StudyReadError::OutOfOrder(_))
    ));
    assert!(matches!(
        bad(&|r| r[4] = r[3].clone()),
        Err(StudyReadError::Mismatch(_))
    ));
    assert!(matches!(
        bad(&|r| r[2].slot = "b99-zzz".into()),
        Err(StudyReadError::Unknown(_))
    ));
    assert!(
        matches!(
            bad(&|r| r[1].reason = Some("x".into())),
            Err(StudyReadError::Malformed(_))
        ),
        "valid with a reason"
    );
    assert!(matches!(
        bad(&|r| {
            r[1].valid = false;
            r[1].elapsed_ns = Some(1);
            r[1].reason = Some("x".into());
        }),
        Err(StudyReadError::Malformed(_))
    ));
    assert!(journal::verify_records(&m, base.clone(), &Mode::Resume(CTX.into())).is_ok());
    let mut too_many = base;
    too_many.push(record(1, "baseline", 1, &m.digest()));
    assert!(journal::verify_records(&m, too_many, &Mode::Analyze).is_err());
}

#[test]
fn contexts_must_be_present_equal_and_current_when_resuming() {
    let (_, m) = fresh("j-context");
    let recorded = schedule_records(&m, Some(CTX), |_, _| Some(7));
    assert!(
        journal::verify_records(&m, recorded[..4].to_vec(), &Mode::Resume("other".into())).is_err(),
        "changed boot, kernel, scheduler, CPUs or source"
    );
    let mut mixed = recorded[..4].to_vec();
    mixed[2].context = Some("ctx-2".into());
    assert!(
        journal::verify_records(&m, mixed.clone(), &Mode::Analyze).is_err(),
        "two contexts"
    );
    mixed[2].context = None;
    assert!(
        journal::verify_records(&m, mixed, &Mode::Analyze).is_err(),
        "recorded and unrecorded mixed"
    );
    let legacy = schedule_records(&m, None, |_, _| Some(7));
    let ok = journal::verify_records(&m, legacy[..4].to_vec(), &Mode::Analyze)
        .expect("legacy is readable");
    assert_eq!(ok.legacy_without_context, 4);
    assert!(
        journal::verify_records(&m, legacy[..4].to_vec(), &Mode::Resume(CTX.into())).is_err(),
        "a new run never resumes context-free records"
    );
}

#[test]
fn failed_verification_launches_nothing_and_a_busy_host_stops_before_any_marker() {
    let (dir, m) = fresh("j-nolaunch");
    committed(&dir, &m, 2);
    let fake = Fake::ok();
    assert!(run_with(&m, &dir, "another-context", &safe, &fake).is_err());
    assert_eq!((fake.prepared.get(), fake.ran.get()), (0, 0));
    let busy = || HostState::Busy("game".into());
    let incomplete = || HostState::Incomplete("cannot read /proc".into());
    for host in [&busy as &dyn Fn() -> HostState, &incomplete] {
        let fake = Fake::ok();
        assert!(run_with(&m, &dir, CTX, host, &fake).is_err());
        assert_eq!((fake.prepared.get(), fake.ran.get()), (0, 0));
    }
    assert_eq!(
        fs::read_dir(dir.join("slots")).expect("slots").count(),
        4,
        "no new marker or record was written"
    );
}

#[test]
fn an_invalid_slot_and_a_harness_error_are_recorded_not_repeated() {
    let (dir, m) = fresh("j-invalid");
    let fake = Fake {
        prepared: Cell::new(0),
        ran: Cell::new(0),
        outcome: Box::new(|slot| match slot.block {
            1 => Ok(Err("bad role readback: cpus differ".into())),
            2 => Err(anyhow::anyhow!("systemd-run is missing")),
            _ => done(5),
        }),
    };
    run_with(&m, &dir, CTX, &safe, &fake).expect("run");
    let records = resume(&dir, &m).expect("verified").records;
    assert_eq!(records.len(), 30);
    assert!(
        records
            .iter()
            .filter(|r| r.block <= 2)
            .all(|r| !r.valid && r.reason.is_some() && r.elapsed_ns.is_none())
    );
    assert!(records.iter().any(|r| {
        r.reason
            .as_deref()
            .is_some_and(|w| w.contains("bad role readback"))
    }));
    assert!(records.iter().any(|r| {
        r.reason
            .as_deref()
            .is_some_and(|w| w.contains("harness error"))
    }));
    let again = Fake::ok();
    run_with(&m, &dir, CTX, &safe, &again).expect("resume");
    assert_eq!(again.ran.get(), 0, "invalid slots are not retried");
}

#[test]
fn a_legacy_study_is_inspected_read_only_with_unchanged_bytes_and_never_resumed() {
    let (dir, m) = fresh("j-legacy");
    let records = schedule_records(&m, None, |_, _| Some(9));
    write_legacy(&dir, &records);
    let before = fs::read(dir.join("results.jsonl")).expect("bytes");
    let report = super::analyze::analyze(&m, &dir).expect("legacy analysis");
    assert!(
        report["cross_context_comparison"]
            .as_str()
            .is_some_and(|c| c.starts_with("LegacyNoContext"))
    );
    assert_eq!(
        fs::read(dir.join("results.jsonl")).expect("bytes"),
        before,
        "the capture is never rewritten"
    );
    let fake = Fake::ok();
    assert!(
        run_with(&m, &dir, CTX, &safe, &fake).is_err(),
        "a legacy store is read-only"
    );
    assert_eq!(fake.ran.get(), 0);
}
