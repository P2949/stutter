use std::fs;

use super::*;

fn fake_process(root: &Path, pid: u32, comm: &str, cpu_ticks: u64, start: u64, flags: u64) {
    let dir = root.join(pid.to_string());
    fs::create_dir_all(dir.join("task").join(pid.to_string())).unwrap();
    // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime
    // stime cutime cstime priority nice threads itrealvalue starttime
    let stat = format!(
        "{pid} ({comm}) R 1 1 1 0 -1 {flags} 0 0 0 0 {cpu_ticks} 0 0 0 20 0 1 0 {start} 0 0\n"
    );
    fs::write(dir.join("stat"), &stat).unwrap();
    fs::write(dir.join("task").join(pid.to_string()).join("stat"), stat).unwrap();
    fs::write(dir.join("cmdline"), format!("/usr/bin/{comm}\0")).unwrap();
}

fn owner() -> u32 {
    fs::metadata("/proc/self").unwrap().uid()
}

/// Samples `rounds` times, advancing the fake CPU counter of every process by `ticks_per_round`
/// (100 ticks per second, 300 ms between samples, so 30 ticks is a full CPU).
fn at(round: u64) -> std::time::Instant {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *BASE.get_or_init(std::time::Instant::now) + Duration::from_millis(300 * round)
}

fn run_rounds(
    root: &Path,
    sampler: &mut OffenderSampler,
    pids: &[(u32, &str)],
    ticks_per_round: u64,
    rounds: u32,
) -> Vec<OffenderProcess> {
    let mut last = Vec::new();
    for round in 0..=rounds {
        for (pid, comm) in pids {
            fake_process(root, *pid, comm, ticks_per_round * u64::from(round), 7, 0);
        }
        last = sampler.sample_at(
            at(u64::from(round)),
            root,
            100,
            owner(),
            &BTreeSet::new(),
            None,
        );
    }
    last
}

#[test]
fn a_process_is_an_offender_only_after_it_stays_hot_for_several_samples() {
    let root = crate::test_support::TestRoot::new("offenders-sustained");
    let mut sampler = OffenderSampler::default();
    let seen = run_rounds(root.path(), &mut sampler, &[(500, "hog")], 30, 2);
    assert!(
        seen.is_empty(),
        "two hot samples are not sustained: {seen:?}"
    );
    let mut sampler = OffenderSampler::default();
    let seen = run_rounds(root.path(), &mut sampler, &[(500, "hog")], 30, 5);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].pid.as_u32(), 500);
    assert_eq!(seen[0].starttime_ticks, 7);
    assert!(seen[0].cpu_percent >= OFFENDER_MIN_CPU_PERCENT);
    assert_eq!(seen[0].tasks.len(), 1);
    assert_eq!(seen[0].tasks[0].starttime_ticks, Some(7));
}

#[test]
fn an_idle_process_is_never_reported() {
    let root = crate::test_support::TestRoot::new("offenders-idle");
    let mut sampler = OffenderSampler::default();
    assert!(run_rounds(root.path(), &mut sampler, &[(501, "idle")], 1, 6).is_empty());
}

#[test]
fn kernel_threads_excluded_pids_and_replaced_processes_are_not_reported() {
    let root = crate::test_support::TestRoot::new("offenders-excluded");
    let mut sampler = OffenderSampler::default();
    for round in 0..=6u64 {
        fake_process(root.path(), 600, "kworker", 30 * round, 7, PF_KTHREAD);
        fake_process(root.path(), 601, "focused", 30 * round, 7, 0);
        // The same pid is reused by a different process (new start time) every round.
        fake_process(root.path(), 602, "recycled", 30 * round, 7 + round, 0);
        let seen = sampler.sample_at(
            at(round),
            root.path(),
            100,
            owner(),
            &BTreeSet::from([601]),
            None,
        );
        assert!(seen.is_empty(), "round {round}: {seen:?}");
    }
}

#[test]
fn another_users_process_and_protected_infrastructure_are_left_alone() {
    let root = crate::test_support::TestRoot::new("offenders-protected");
    let mut sampler = OffenderSampler::default();
    let foreign = owner().wrapping_add(1);
    let mut last = Vec::new();
    for round in 0..=6u64 {
        fake_process(root.path(), 700, "pipewire", 30 * round, 7, 0);
        fake_process(root.path(), 701, "hog", 30 * round, 7, 0);
        last = sampler.sample_at(at(round), root.path(), 100, foreign, &BTreeSet::new(), None);
    }
    assert!(
        last.is_empty(),
        "a foreign owner is never an offender: {last:?}"
    );
    let mut sampler = OffenderSampler::default();
    let mut last = Vec::new();
    for round in 0..=6u64 {
        fake_process(root.path(), 700, "pipewire", 30 * round, 7, 0);
        last = sampler.sample_at(at(round), root.path(), 100, owner(), &BTreeSet::new(), None);
    }
    assert!(
        last.is_empty(),
        "audio infrastructure is protected: {last:?}"
    );
}

#[test]
fn a_validation_scope_reports_only_descendants_of_its_harness() {
    let root = crate::test_support::TestRoot::new("offenders-scope");
    let mut sampler = OffenderSampler::default();
    let mut last = Vec::new();
    for round in 0..=6u64 {
        // fake_process gives every process parent 1; 4242 is the harness that started none of them.
        fake_process(root.path(), 800, "hog", 30 * round, 7, 0);
        last = sampler.sample_at(
            at(round),
            root.path(),
            100,
            owner(),
            &BTreeSet::new(),
            Some(4242),
        );
    }
    assert!(last.is_empty(), "{last:?}");
}
