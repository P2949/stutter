use std::fs;

use super::*;
use crate::integrations::receipt::{ReceiptBook, ReceiptDenial};

fn task(name: &str, nice: i32, policy: u32, cpus: &str) -> (std::path::PathBuf, u32) {
    let root = std::env::temp_dir().join(format!("stutter-intent-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("77");
    fs::create_dir_all(&dir).unwrap();
    // 38 fields after the comm: nice is index 16, policy index 38.
    let mut fields = vec!["0".to_owned(); 39];
    fields[0] = "S".into();
    fields[16] = nice.to_string();
    fields[38] = policy.to_string();
    fs::write(
        dir.join("stat"),
        format!("77 (game) {}\n", fields.join(" ")),
    )
    .unwrap();
    fs::write(
        dir.join("status"),
        format!("Name:\tgame\nCpus_allowed_list:\t{cpus}\n"),
    )
    .unwrap();
    (root, 77)
}

#[test]
fn nice_policy_and_the_cpu_mask_are_read_back_from_the_kernel_files() {
    let (root, pid) = task("read", -5, 1, "2-3");
    let intent = read_task_intent(&root, pid).unwrap();
    assert_eq!((intent.nice, intent.policy), (-5, SchedPolicy::Fifo));
    assert_eq!(intent.cpus_allowed, vec![2, 3]);
    assert!(intent.policy.is_real_time());
    assert!(read_task_intent(&root, 999).is_none());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_a_restricted_mask_or_a_set_nice_value_is_someones_intent() {
    let online = [0, 1, 2, 3];
    let free = TaskIntent {
        cpus_allowed: vec![0, 1, 2, 3],
        nice: 0,
        policy: SchedPolicy::Normal,
    };
    let report = classify(&free, &online);
    assert_eq!(
        (
            report.affinity_restricted,
            report.nice_set,
            report.real_time
        ),
        (false, false, false)
    );
    assert!(user_receipts(1, 1, &free, &online, 0, 10).is_empty());
    let pinned = TaskIntent {
        cpus_allowed: vec![2, 3],
        nice: 5,
        policy: SchedPolicy::Normal,
    };
    assert!(classify(&pinned, &online).affinity_restricted && classify(&pinned, &online).nice_set);
    assert_eq!(user_receipts(1, 1, &pinned, &online, 0, 10).len(), 2);
    let rt = TaskIntent {
        cpus_allowed: vec![0, 1, 2, 3],
        nice: 0,
        policy: SchedPolicy::Deadline,
    };
    assert!(classify(&rt, &online).real_time);
    assert!(
        user_receipts(1, 1, &rt, &online, 0, 10).is_empty(),
        "a real-time policy is a diagnostic, never an owned attribute"
    );
}

#[test]
fn a_user_set_attribute_denies_a_mutation_through_its_receipt() {
    let online = [0, 1, 2, 3];
    let intent = TaskIntent {
        cpus_allowed: vec![2],
        nice: 5,
        policy: SchedPolicy::Normal,
    };
    let mut book = ReceiptBook::default();
    for receipt in user_receipts(42, 7, &intent, &online, 100, 1_000) {
        book.record(receipt);
    }
    let nice = ResourceKey::Task {
        pid: 42,
        start_ticks: 7,
        attribute: TaskAttribute::Nice,
    };
    assert_eq!(
        book.permit(&nice, 200, "5"),
        Err(ReceiptDenial::Owner(OwnerClass::UserExplicit))
    );
    // A different task with the same pid but another start time has no such receipt.
    let recycled = ResourceKey::Task {
        pid: 42,
        start_ticks: 8,
        attribute: TaskAttribute::Nice,
    };
    assert_eq!(
        book.permit(&recycled, 200, "0"),
        Err(ReceiptDenial::NoReceipt)
    );
}

#[test]
fn the_policy_numbers_map_to_the_linux_policies() {
    assert_eq!(SchedPolicy::from_raw(0), SchedPolicy::Normal);
    assert_eq!(SchedPolicy::from_raw(2), SchedPolicy::RoundRobin);
    assert_eq!(SchedPolicy::from_raw(5), SchedPolicy::Idle);
    assert_eq!(SchedPolicy::from_raw(99), SchedPolicy::Other(99));
    assert!(!SchedPolicy::Batch.is_real_time());
}

#[test]
fn a_target_with_a_non_default_nice_value_is_a_conflict_for_a_nice_candidate() {
    let mut nice = std::collections::BTreeMap::new();
    nice.insert(10, 0);
    nice.insert(11, 5);
    nice.insert(12, -3);
    assert_eq!(explicit_nice_conflict(&nice, &[10]), None);
    assert_eq!(explicit_nice_conflict(&nice, &[10, 11, 12]), Some(11));
    assert_eq!(
        explicit_nice_conflict(&nice, &[99]),
        None,
        "an unknown task is not a conflict"
    );
    assert_eq!(
        crate::autotune::planning::denial::CandidateDenyReason::UserExplicitSetting.reason_code(),
        "user_explicit_setting"
    );
}
