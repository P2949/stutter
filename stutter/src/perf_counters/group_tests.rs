use super::*;

#[test]
fn six_event_readback_preserves_optional_branch_semantics() {
    let delta = decode_interval(
        &[
            PerfEventKind::Cycles,
            PerfEventKind::Instructions,
            PerfEventKind::CacheMisses,
            PerfEventKind::CacheReferences,
            PerfEventKind::Branches,
            PerfEventKind::BranchMisses,
        ],
        [6, 100, 100, 1000, 2000, 50, 1000, 500, 25],
        &[],
    );
    assert_eq!(delta.ipc, Some(2.0));
    assert_eq!(delta.branches, Some(500));
    assert_eq!(delta.branch_misses, Some(25));
    assert_eq!(delta.branch_miss_rate, Some(0.05));
    assert!(!delta.multiplexed);
    assert!(!delta.scaled);
}

#[test]
fn unsupported_events_are_missing_and_not_zeroes() {
    let delta = decode_interval(
        &[
            PerfEventKind::Cycles,
            PerfEventKind::Instructions,
            PerfEventKind::CacheMisses,
        ],
        [3, 100, 100, 1000, 2000, 50, 0, 0, 0],
        &["branches".into(), "branch_misses".into()],
    );
    assert_eq!(delta.branches, None);
    assert_eq!(delta.branch_misses, None);
    assert_eq!(delta.branch_miss_rate, None);
    assert_eq!(delta.unsupported_events, ["branches", "branch_misses"]);
}

#[test]
fn oversized_syscall_payload_is_refused_before_reading() {
    let err = syscall::read_counters(-1, &mut [0; 1], 16).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
#[ignore = "read-only live own-thread PMU verification; requires perf permission and hardware events"]
fn live_own_thread_group_intervals() {
    for collect_branches in [false, true] {
        let config = CpuPerfConfig {
            include_kernel: false,
            max_tasks: 1,
            collect_cache_refs: true,
            collect_branches,
        };
        let mut group = PerfCounterGroup::open(Tid::new(crate::syscall::gettid()), &config)
            .unwrap_or_else(|error| panic!("BlockedResource perf_event_open: {}", error.message));
        for round in 0..3 {
            let mut checksum = round as u64;
            // A sub-millisecond burst can finish before a multiplexed group is scheduled. This
            // bounded own-thread check measures 50ms of work; it does not relax any quality gate.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
            while std::time::Instant::now() < deadline {
                for index in 0..100_000_u64 {
                    checksum = std::hint::black_box(checksum.wrapping_mul(31).wrapping_add(index));
                }
            }
            std::hint::black_box(checksum);
            let delta = group.sample_interval().unwrap();
            println!(
                "{}",
                serde_json::json!({"collect_branches":collect_branches,"round":round,"delta":delta})
            );
            assert!(delta.unavailable_reason.is_none());
            assert!(delta.time_enabled_ns.unwrap() >= delta.time_running_ns.unwrap());
            assert!(delta.time_running_ns.unwrap() > 0);
            assert!(delta.cycles.unwrap() > 0);
            assert!(delta.instructions.unwrap() > 0);
        }
    }
}
