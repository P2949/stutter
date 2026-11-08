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

#[test]
fn ionice_uclamp_and_memory_policy_defaults_are_nobodys_decision_and_anything_else_is() {
    use std::collections::BTreeMap;

    use crate::actions::uclamp::UclampValues;

    let mut io = BTreeMap::new();
    io.insert(1, "none".to_owned());
    io.insert(2, "best-effort:2".to_owned());
    io.insert(3, "idle".to_owned());
    io.insert(4, "best-effort:4".to_owned());
    assert_eq!(explicit_ioprio_conflict(&io, &[1]), None);
    assert_eq!(explicit_ioprio_conflict(&io, &[1, 2]), Some(2));
    assert_eq!(explicit_ioprio_conflict(&io, &[3]), Some(3));
    assert_eq!(explicit_ioprio_conflict(&io, &[9]), None);
    assert_eq!(
        explicit_ioprio_conflict(&io, &[4]),
        None,
        "the kernel default level is nobody's decision"
    );

    let mut clamps = BTreeMap::new();
    clamps.insert(
        1,
        UclampValues {
            sched_util_min: Some(0),
            sched_util_max: Some(1024),
        },
    );
    clamps.insert(
        2,
        UclampValues {
            sched_util_min: Some(300),
            sched_util_max: Some(1024),
        },
    );
    clamps.insert(
        3,
        UclampValues {
            sched_util_min: Some(0),
            sched_util_max: Some(512),
        },
    );
    clamps.insert(
        4,
        UclampValues {
            sched_util_min: None,
            sched_util_max: None,
        },
    );
    assert_eq!(explicit_uclamp_conflict(&clamps, &[1, 4]), None);
    assert_eq!(explicit_uclamp_conflict(&clamps, &[1, 2]), Some(2));
    assert_eq!(explicit_uclamp_conflict(&clamps, &[3]), Some(3));

    let default_only = "7f00 default anon=3 dirty=3 N0=3\n7f10 default file=/lib/x N0=1\n";
    assert!(!mempolicy_is_explicit(default_only));
    let bound = "7f00 default anon=3\n7f20 bind:0 anon=9 N0=9\n";
    assert!(mempolicy_is_explicit(bound));
    assert!(mempolicy_is_explicit("7f00 interleave:0-1 anon=2\n"));
    assert!(!mempolicy_is_explicit(""));
}

#[test]
fn memory_policy_and_cgroup_settings_are_read_from_their_files_and_unknown_is_not_default() {
    let base = std::env::temp_dir().join(format!("stutter-intent-files-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("5")).unwrap();
    fs::write(base.join("5/numa_maps"), "7f00 bind:0 anon=1\n").unwrap();
    assert_eq!(read_mempolicy_explicit(&base, 5), Some(true));
    fs::write(base.join("5/numa_maps"), "7f00 default anon=1\n").unwrap();
    assert_eq!(read_mempolicy_explicit(&base, 5), Some(false));
    assert_eq!(
        read_mempolicy_explicit(&base, 6),
        None,
        "no file means unknown, never default"
    );

    let group = base.join("group");
    fs::create_dir_all(&group).unwrap();
    for (name, text) in [
        ("cpu.weight", "100\n"),
        ("cpu.max", "max 100000\n"),
        ("cpu.idle", "0\n"),
    ] {
        fs::write(group.join(name), text).unwrap();
    }
    assert_eq!(read_cgroup_intent(&group), CgroupIntent::default());
    fs::write(group.join("cpu.weight"), "500\n").unwrap();
    fs::write(group.join("cpu.max"), "50000 100000\n").unwrap();
    fs::write(group.join("cpu.idle"), "1\n").unwrap();
    assert_eq!(
        read_cgroup_intent(&group),
        CgroupIntent {
            cpu_weight_set: true,
            cpu_max_set: true,
            cpu_idle_set: true
        }
    );
    assert_eq!(
        read_cgroup_intent(&base.join("missing")),
        CgroupIntent::default()
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn this_process_intent_is_read_from_the_real_proc() {
    let intent = read_task_intent(std::path::Path::new("/proc"), std::process::id()).unwrap();
    assert!(!intent.cpus_allowed.is_empty());
    assert!(
        !intent.policy.is_real_time(),
        "the test runner is not a real-time task"
    );
    let online = crate::topology::TopologyModel::read()
        .unwrap()
        .online_cpu_ids();
    // A normal process on a normal host is nobody's explicit decision.
    let report = classify(&intent, &online);
    assert!(!report.real_time);
}
