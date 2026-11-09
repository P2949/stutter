use std::{fs, os::unix::fs::MetadataExt};

use super::*;

fn mount(name: &str, closids: u32) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-resctrl-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("info/L3")).unwrap();
    fs::write(root.join("info/L3/cbm_mask"), "fff\n").unwrap();
    fs::write(root.join("info/L3/min_cbm_bits"), "2\n").unwrap();
    fs::write(root.join("info/L3/num_closids"), format!("{closids}\n")).unwrap();
    fs::write(root.join("schemata"), "L3:0=fff;1=fff\nMB:0=100;1=100\n").unwrap();
    root
}

fn request(mask: u64) -> CatRequest {
    CatRequest {
        name: "trial1".into(),
        mask,
        ..Default::default()
    }
}

#[test]
fn a_prepared_group_commits_with_readback_and_restores_by_removal() {
    let root = mount("cycle", 4);
    let prepared = prepare(&root, &request(0x0f0)).unwrap();
    assert_eq!(prepared.schemata(), "L3:0=f0;1=f0");
    assert!(
        !root.join("stutter-cat-trial1").exists(),
        "prepare writes nothing"
    );
    let committed = prepared.commit().unwrap();
    assert_eq!(
        fs::read_to_string(root.join("stutter-cat-trial1/schemata"))
            .unwrap()
            .trim(),
        "L3:0=f0;1=f0"
    );
    assert_eq!(committed.restore(), CatRestore::Removed);
    assert!(!root.join("stutter-cat-trial1").exists());
    assert_eq!(
        fs::read_to_string(root.join("schemata")).unwrap(),
        "L3:0=fff;1=fff\nMB:0=100;1=100\n",
        "the default group is untouched"
    );
}

#[test]
fn bad_masks_names_and_absent_hardware_are_denied_before_any_write() {
    let root = mount("deny", 4);
    for mask in [0, 0b101, 0b1, 0x1000, 0x3_0000_0000_u64] {
        assert_eq!(
            prepare(&root, &request(mask)).unwrap_err(),
            CatDenial::BadMask,
            "{mask:#x}"
        );
    }
    let bad_name = CatRequest {
        name: "../x".into(),
        mask: 0xf0,
        ..Default::default()
    };
    assert_eq!(prepare(&root, &bad_name).unwrap_err(), CatDenial::BadName);
    let absent = std::env::temp_dir().join("stutter-resctrl-not-mounted");
    assert_eq!(
        prepare(&absent, &request(0xf0)).unwrap_err(),
        CatDenial::Unavailable
    );
    fs::write(root.join("schemata"), "MB:0=100\n").unwrap();
    assert_eq!(
        prepare(&root, &request(0xf0)).unwrap_err(),
        CatDenial::RootUnreadable
    );
}

#[test]
fn a_depleted_closid_pool_and_an_existing_group_are_refused() {
    let root = mount("closid", 2);
    fs::create_dir(root.join("other")).unwrap();
    fs::write(root.join("other/schemata"), "L3:0=fff;1=fff\n").unwrap();
    assert_eq!(
        prepare(&root, &request(0xf0)).unwrap_err(),
        CatDenial::NoFreeClosid
    );
    let roomy = mount("exists", 8);
    fs::create_dir(roomy.join("stutter-cat-trial1")).unwrap();
    assert_eq!(
        prepare(&roomy, &request(0xf0)).unwrap_err(),
        CatDenial::GroupExists
    );
}

#[test]
fn restore_never_removes_a_replaced_group() {
    let root = mount("replaced", 4);
    let committed = prepare(&root, &request(0xf0)).unwrap().commit().unwrap();
    // Another owner's directory takes the path while ours is still alive (distinct inode).
    fs::rename(root.join("stutter-cat-trial1"), root.join("moved-away")).unwrap();
    fs::create_dir(root.join("stutter-cat-trial1")).unwrap();
    assert_eq!(committed.restore(), CatRestore::UnsafeToRestore);
    assert!(
        root.join("stutter-cat-trial1").exists(),
        "the other group survives"
    );
}

#[test]
fn restore_reports_an_already_absent_group() {
    let root = mount("absent", 4);
    let committed = prepare(&root, &request(0xf0)).unwrap().commit().unwrap();
    fs::remove_dir_all(root.join("stutter-cat-trial1")).unwrap();
    assert_eq!(committed.restore(), CatRestore::Absent);
}

#[test]
fn a_group_whose_identity_cannot_be_read_after_creation_is_removed_not_leaked() {
    let root = mount("identity-fault", 4);
    let prepared = prepare(&root, &request(0x0f0)).unwrap();
    let error = prepared
        .commit_with(|_| Err(io::Error::other("injected metadata failure")))
        .unwrap_err();
    assert_eq!(error, CatCommitError::IdentityUnreadable { removed: true });
    assert!(
        !root.join("stutter-cat-trial1").exists(),
        "the created group was left behind"
    );
}

fn with_mba(root: &Path) {
    fs::create_dir_all(root.join("info/MB")).unwrap();
    fs::write(root.join("info/MB/min_bandwidth"), "10\n").unwrap();
    fs::write(root.join("info/MB/bandwidth_gran"), "10\n").unwrap();
}

fn mba_request(percent: u32, tasks: Vec<CatTask>) -> CatRequest {
    CatRequest {
        name: "trial1".into(),
        mask: 0x0f0,
        mba_percent: Some(percent),
        tasks,
    }
}

#[test]
fn a_bandwidth_limit_is_written_read_back_and_removed_with_the_group() {
    let root = mount("mba", 4);
    with_mba(&root);
    let prepared = prepare(&root, &mba_request(50, Vec::new())).unwrap();
    assert_eq!(
        prepared.schemata_lines(),
        ["L3:0=f0;1=f0".to_owned(), "MB:0=50;1=50".to_owned()]
    );
    let committed = prepared.commit().unwrap();
    let text = fs::read_to_string(root.join("stutter-cat-trial1/schemata")).unwrap();
    assert!(text.contains("MB:0=50;1=50"), "{text}");
    assert_eq!(committed.restore(), CatRestore::Removed);
    assert_eq!(
        fs::read_to_string(root.join("schemata")).unwrap(),
        "L3:0=fff;1=fff\nMB:0=100;1=100\n",
        "the default group is untouched"
    );
}

#[test]
fn bandwidth_is_denied_when_absent_out_of_range_or_off_granularity() {
    let root = mount("mba-deny", 4);
    assert_eq!(
        prepare(&root, &mba_request(50, Vec::new())).unwrap_err(),
        CatDenial::MbaUnavailable
    );
    with_mba(&root);
    for percent in [0, 5, 55, 101, 200] {
        assert_eq!(
            prepare(&root, &mba_request(percent, Vec::new())).unwrap_err(),
            CatDenial::BadBandwidth,
            "{percent}"
        );
    }
    // A root already in MBps mode is not a percentage controller.
    fs::write(root.join("schemata"), "L3:0=fff\nMB:0=2048\n").unwrap();
    assert_eq!(
        prepare(&root, &mba_request(50, Vec::new())).unwrap_err(),
        CatDenial::MbaUnavailable
    );
}

fn inspect(task: u32) -> Option<(u64, Vec<u32>)> {
    match task {
        4242 => Some((777, vec![4242, 4243])),
        4300 => Some((888, vec![4300])),
        _ => None,
    }
}

#[test]
fn named_processes_move_into_the_group_by_live_identity_every_thread() {
    let root = mount("tasks", 4);
    fs::write(root.join("tasks"), "1\n4242\n4243\n4300\n").unwrap();
    let ok = CatRequest {
        tasks: vec![CatTask {
            task: 4242,
            start_ticks: 777,
        }],
        ..request(0x0f0)
    };
    let prepared = prepare_with(&root, &ok, &inspect).unwrap();
    assert_eq!(
        prepared.threads,
        [4242, 4243],
        "every thread of the process"
    );
    let committed = prepared.commit().unwrap();
    // A plain file keeps only the last write; the kernel accumulates members.
    let members = fs::read_to_string(root.join("stutter-cat-trial1/tasks")).unwrap();
    assert!(members.contains("4243"), "{members}");
    assert!(!members.contains("4300"), "an unnamed process was moved");
    assert_eq!(committed.restore(), CatRestore::Removed);

    // A reused pid (different start time), an unknown pid and a task owned by another group.
    for (task, ticks, denial) in [
        (4242, 1, CatDenial::TaskIdentity),
        (9999, 1, CatDenial::TaskIdentity),
    ] {
        let bad = CatRequest {
            tasks: vec![CatTask {
                task,
                start_ticks: ticks,
            }],
            ..request(0x0f0)
        };
        assert_eq!(prepare_with(&root, &bad, &inspect).unwrap_err(), denial);
    }
    fs::write(root.join("tasks"), "1\n4242\n").unwrap();
    let taken = CatRequest {
        tasks: vec![CatTask {
            task: 4242,
            start_ticks: 777,
        }],
        ..request(0x0f0)
    };
    assert_eq!(
        prepare_with(&root, &taken, &inspect).unwrap_err(),
        CatDenial::TaskNotInDefaultGroup,
        "thread 4243 is not in the default group, so the process is not taken"
    );
    let many = CatRequest {
        tasks: vec![
            CatTask {
                task: 4300,
                start_ticks: 888
            };
            MAX_ASSIGNED_TASKS + 1
        ],
        ..request(0x0f0)
    };
    assert_eq!(
        prepare_with(&root, &many, &inspect).unwrap_err(),
        CatDenial::TooManyTasks
    );
}

#[test]
fn a_failed_assignment_removes_the_group_and_reports_it() {
    let root = mount("assign-fail", 4);
    fs::write(root.join("tasks"), "1\n4300\n").unwrap();
    let req = CatRequest {
        tasks: vec![CatTask {
            task: 4300,
            start_ticks: 888,
        }],
        ..request(0x0f0)
    };
    let prepared = prepare_with(&root, &req, &inspect).unwrap();
    // The kernel refuses the write: a directory stands where the `tasks` file would be written.
    // (The group is created by commit, so make `tasks` unwritable by pre-creating it as a dir.)
    let group = root.join("stutter-cat-trial1");
    let error = prepared
        .commit_with(|path| {
            fs::create_dir_all(path.join("tasks"))?;
            fs::metadata(path).map(|meta| (meta.dev(), meta.ino()))
        })
        .unwrap_err();
    assert_eq!(
        error,
        CatCommitError::TaskAssignmentFailed { removed: true }
    );
    assert!(!group.exists(), "the group was left behind");
}
