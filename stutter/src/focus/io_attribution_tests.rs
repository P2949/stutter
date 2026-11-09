use super::*;
use crate::{focus::test_support::foreground_test_process, process_tree::TaskClass};

fn process(pid: u32, class: TaskClass, read: u64, write: u64, start: Option<u64>) -> FocusProcess {
    let mut process = foreground_test_process(pid, 1, class);
    process.read_bytes_delta = read;
    process.write_bytes_delta = write;
    process.starttime_ticks = start;
    process.is_foreground_window_process = false;
    process
}

#[test]
fn offenders_are_ranked_by_bytes_moved_with_exact_identity_and_shares() {
    let processes = [
        process(10, TaskClass::Compiler, 1_000, 2_000, Some(111)),
        process(11, TaskClass::Compiler, 0, 7_000, Some(222)),
        process(12, TaskClass::Compiler, 0, 0, Some(333)),
    ];
    let ranked = rank_io_offenders(&processes, 1, 10);
    assert_eq!(
        ranked
            .iter()
            .map(|o| (o.pid, o.starttime_ticks))
            .collect::<Vec<_>>(),
        vec![(11, 222), (10, 111)],
        "idle processes are not offenders"
    );
    assert_eq!(ranked[0].share_permille, 700);
    assert_eq!(ranked[1].share_permille, 300);
    assert_eq!(rank_io_offenders(&processes, 1, 1).len(), 1);
    assert!(rank_io_offenders(&processes, 8_000, 10).is_empty());
}

#[test]
fn a_process_without_a_start_time_is_never_attributed_and_the_foreground_is_protected() {
    let mut focused = process(20, TaskClass::Compiler, 500, 500, Some(1));
    focused.is_foreground_window_process = true;
    let processes = [
        process(21, TaskClass::Compiler, 9_999, 0, None),
        focused,
        process(22, TaskClass::Game, 5, 5, Some(2)),
        process(23, TaskClass::Compiler, 1, 1, Some(3)),
    ];
    let ranked = rank_io_offenders(&processes, 1, 10);
    assert!(ranked.iter().all(|o| o.pid != 21));
    let flag = |pid| ranked.iter().find(|o| o.pid == pid).unwrap().protected;
    assert!(flag(20) && flag(22) && !flag(23));
}

#[test]
fn ties_break_by_pid_and_totals_never_overflow() {
    let processes = [
        process(31, TaskClass::Compiler, u64::MAX, u64::MAX, Some(1)),
        process(30, TaskClass::Compiler, u64::MAX, u64::MAX, Some(2)),
    ];
    let ranked = rank_io_offenders(&processes, 1, 10);
    assert_eq!(
        ranked.iter().map(|o| o.pid).collect::<Vec<_>>(),
        vec![30, 31]
    );
    assert!(ranked.iter().all(|o| o.share_permille <= 1000));
}
