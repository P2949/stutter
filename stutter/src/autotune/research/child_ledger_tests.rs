use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchAction, ResearchTool,
    child_study::{Profile, Stage, StudyResult, plan},
    tests::intent as base_intent,
};

fn studies() -> Vec<ChildStudy> {
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    plan(&intent).unwrap()
}

fn dir() -> (tempfile::TempDir, File) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("child-studies");
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = File::open(&path).unwrap();
    (root, file)
}

fn reopen(root: &tempfile::TempDir, plan: Vec<ChildStudy>) -> anyhow::Result<ChildLedger> {
    ChildLedger::open(
        File::open(root.path().join("child-studies"))?,
        crate::syscall::geteuid(),
        plan,
    )
}

#[test]
fn the_funnel_resumes_after_a_restart_and_never_repeats_or_reorders_a_decision() {
    let (root, handle) = dir();
    let plan = studies();
    let uid = crate::syscall::geteuid();
    let mut ledger = ChildLedger::open(handle, uid, plan.clone()).unwrap();
    let first = ledger.next_runnable(100, &|_| None).unwrap().unwrap();
    assert_eq!((first.profile, first.stage), (Profile::Cpu, Stage::Screen));
    ledger.record(&first, StudyResult::Qualified, "").unwrap();
    // A restarted controller sees the same state and picks the next study in funnel order.
    let mut restarted = reopen(&root, plan.clone()).unwrap();
    assert!(
        restarted
            .record(&first, StudyResult::Failed, "again")
            .is_err()
    );
    let second = restarted.next_runnable(100, &|_| None).unwrap().unwrap();
    assert_eq!(
        (second.profile, second.stage),
        (Profile::Gpu, Stage::Screen)
    );
}

#[test]
fn blocked_budget_and_unqualified_predecessors_are_recorded_not_dropped() {
    let (root, handle) = dir();
    let plan = studies();
    let mut ledger = ChildLedger::open(handle, crate::syscall::geteuid(), plan.clone()).unwrap();
    // Nothing can apply a graphics profile: every study that needs one is blocked with that
    // reason, and the dependents of a blocked screen are blocked by their predecessor.
    let none = ledger
        .next_runnable(100, &|_| Some("BlockedProfileApplier"))
        .unwrap();
    assert!(none.is_none());
    let all = ledger.dispositions();
    assert_eq!(all.len(), plan.len());
    assert!(all.iter().all(|d| d.result == Some(StudyResult::Blocked)));
    assert_eq!(all[0].reason, "BlockedProfileApplier");
    assert!(
        all.iter()
            .skip(2)
            .any(|d| d.reason == "PredecessorNotQualified")
    );
    // The decisions are durable.
    assert!(
        reopen(&root, plan)
            .unwrap()
            .dispositions()
            .iter()
            .all(|d| d.result.is_some())
    );
}

#[test]
fn an_insufficient_budget_defers_a_study_and_everything_after_it_is_still_accounted_for() {
    let (_root, handle) = dir();
    let plan = studies();
    let mut ledger = ChildLedger::open(handle, crate::syscall::geteuid(), plan.clone()).unwrap();
    // The screens need two attempts each (two tools); one attempt fits none of them.
    assert!(ledger.next_runnable(1, &|_| None).unwrap().is_none());
    let all = ledger.dispositions();
    assert_eq!(all[0].result, Some(StudyResult::DeferredBudget));
    assert!(all.iter().all(|d| d.result.is_some()));
}

#[test]
fn a_tampered_gapped_foreign_or_duplicated_ledger_is_refused() {
    let (root, handle) = dir();
    let plan = studies();
    let uid = crate::syscall::geteuid();
    let mut ledger = ChildLedger::open(handle, uid, plan.clone()).unwrap();
    ledger.record(&plan[0], StudyResult::Qualified, "").unwrap();
    ledger.record(&plan[1], StudyResult::Qualified, "").unwrap();
    let base = root.path().join("child-studies");
    assert!(reopen(&root, plan.clone()).is_ok());
    // Editing a record breaks the next record's chain.
    let first = base.join("0000.json");
    let text = std::fs::read_to_string(&first).unwrap();
    std::fs::write(&first, text.replace("qualified", "failed")).unwrap();
    assert!(reopen(&root, plan.clone()).is_err());
    std::fs::write(&first, text).unwrap();
    assert!(reopen(&root, plan.clone()).is_ok());
    // A gap, a stray file, and a study outside the plan are refused.
    std::fs::rename(base.join("0001.json"), base.join("0002.json")).unwrap();
    assert!(reopen(&root, plan.clone()).is_err());
    std::fs::rename(base.join("0002.json"), base.join("0001.json")).unwrap();
    std::fs::write(base.join("notes.txt"), b"x").unwrap();
    assert!(reopen(&root, plan.clone()).is_err());
    std::fs::remove_file(base.join("notes.txt")).unwrap();
    assert!(
        reopen(&root, plan[1..].to_vec()).is_err(),
        "plan no longer contains record 0"
    );
}

#[test]
fn unexecuted_studies_are_settled_once_with_their_reason_and_a_missing_ledger_reads_undecided() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    // No ledger yet: everything planned is reported, nothing decided.
    let before = read_dispositions(&campaign, uid, &intent).unwrap();
    assert_eq!(before.len(), plan(&intent).unwrap().len());
    assert!(before.iter().all(|d| d.result.is_none()));
    settle_unexecuted(&campaign, uid, &intent).unwrap();
    // Repeating it (a restart at the end of the campaign) changes nothing and does not fail.
    settle_unexecuted(&campaign, uid, &intent).unwrap();
    let after = read_dispositions(&campaign, uid, &intent).unwrap();
    assert!(after.iter().all(|d| d.result == Some(StudyResult::Blocked)));
    assert!(
        after
            .iter()
            .any(|d| d.profile == Profile::Cpu && d.reason == "BlockedProfileApplier")
    );
    assert!(
        after
            .iter()
            .any(|d| d.profile == Profile::Long && !d.reason.is_empty())
    );
}
