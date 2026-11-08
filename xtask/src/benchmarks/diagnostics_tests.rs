use super::diagnostics::*;

fn value(scores: &super::scores::ImportedScores, label: &str) -> f64 {
    scores
        .scores
        .iter()
        .find(|s| s.label == label)
        .unwrap_or_else(|| panic!("{label} missing"))
        .value
}

#[test]
fn iostat_pidstat_numastat_blkparse_tuna_and_cyclictest_parse_to_non_proof_scores() {
    let iostat = import_iostat("avg-cpu:  %user\n   1.0\n\nDevice  r/s  w/s  await\nsda     10.5  2.0  0.75\nnvme0n1  1.0  3.0  0.10\n").unwrap();
    assert_eq!(value(&iostat, "sda.await"), 0.75);
    assert_eq!(value(&iostat, "nvme0n1.w/s"), 3.0);
    assert!(!iostat.application_proof);
    assert!(iostat.scores.iter().all(|s| s.higher_is_better.is_none()));

    let pidstat = import_pidstat("12:00:01 AM   UID   PID  %usr %system  %CPU  CPU  Command\n12:00:02 AM  1000  4242  12.00   3.00 15.00    2  game\nAverage:     1000  4242  11.00   2.00 13.00    -  game\n").unwrap();
    assert_eq!(value(&pidstat, "game[4242].%usr"), 12.0);
    assert_eq!(value(&pidstat, "game[4242].%CPU"), 15.0);

    let numastat = import_numastat("                  node0      node1\nnuma_hit        1000       2000\nnuma_miss          5          7\n").unwrap();
    assert_eq!(value(&numastat, "numa_miss.node1"), 7.0);

    let blk = import_blkparse("Total (sda):\n Reads Completed:       10,       40KiB\n Writes Completed:       5,       20KiB\nEvents (sda): 1234 entries\n").unwrap();
    assert_eq!(value(&blk, "reads_completed"), 10.0);
    assert_eq!(value(&blk, "events"), 1234.0);

    let tuna = import_tuna("thread  ctxt_switches\n pid SCHED_ rtpri affinity voluntary nonvoluntary cmd\n  1   OTHER    0    0,1   100   5  systemd\n").unwrap();
    assert_eq!(value(&tuna, "systemd[1].voluntary_ctxt_switches"), 100.0);

    let cyclic = import_cyclictest(
        "T: 0 ( 1234) P:95 I:1000 C:  10000 Min:      3 Act:    5 Avg:    4 Max:     21\n",
    )
    .unwrap();
    assert_eq!(value(&cyclic, "t0.max"), 21.0);
    assert_eq!(cyclic.scores[0].higher_is_better, Some(false));
}

#[test]
fn input_without_a_table_is_an_error_never_an_empty_success() {
    for result in [
        import_iostat("hello"),
        import_pidstat("hello"),
        import_numastat("hello"),
        import_blkparse("hello"),
        import_tuna("hello"),
        import_cyclictest("hello"),
        import_iostat("Device r/s\nsda notanumber\n"),
    ] {
        assert!(result.is_err());
    }
}

#[test]
fn histograms_report_buckets_a_total_and_only_a_bucket_bound_for_p99() {
    let bpftrace = import_bpftrace_hist("@usecs:\n[0]       10 |@@@@|\n[1]       80 |@@@@@@@@@@|\n[2, 4)     9 |@|\n[1K, 2K)   1 |\n").unwrap();
    assert_eq!(value(&bpftrace, "total"), 100.0);
    assert_eq!(value(&bpftrace, "bucket_2"), 9.0);
    assert_eq!(
        value(&bpftrace, "p99_bucket_lower_bound"),
        2.0,
        "99 of 100 are below the last bucket"
    );
    assert!(!bpftrace.application_proof);

    let bcc = import_bcc_hist("     usecs               : count     distribution\n         0 -> 1          : 5        |***|\n         2 -> 3          : 95       |*********|\n").unwrap();
    assert_eq!(value(&bcc, "total"), 100.0);
    assert_eq!(value(&bcc, "p99_bucket_lower_bound"), 2.0);
}

#[test]
fn trace_text_counts_events_and_dropped_events_and_junk_is_an_error() {
    let text = "CPU:0 [12 EVENTS DROPPED]\n  game-4242 [002] 100.000001: sched_switch: prev_comm=game prev_pid=4242\n  game-4242 [002] 100.000002: sched_wakeup: comm=x pid=1\n  game-4242 [003] 100.000003: sched_switch: prev_comm=y\n";
    let trace = import_trace_cmd(text).unwrap();
    assert_eq!(value(&trace, "events.sched_switch"), 2.0);
    assert_eq!(value(&trace, "events.sched_wakeup"), 1.0);
    assert_eq!(value(&trace, "events_dropped"), 12.0);

    for bad in [
        import_bpftrace_hist("nothing"),
        import_bpftrace_hist("[0]  0 |\n"),
        import_bcc_hist(""),
        import_trace_cmd("not a trace"),
    ] {
        assert!(bad.is_err());
    }
}

#[test]
fn perf_stat_values_need_a_full_run_and_are_never_extrapolated() {
    let text = "# started\n1000000,,cycles,1000000,100.00,,\n500,,cache-misses,400000,40.00,,\n<not counted>,,instructions,0,0.00,,\n<not supported>,,power/energy-pkg/,0,100.00,,\nbad line\n";
    let stat = import_perf_stat(text).unwrap();
    assert_eq!(value(&stat, "cycles"), 1_000_000.0);
    assert_eq!(value(&stat, "cache-misses.running_percent"), 40.0);
    assert!(
        !stat.scores.iter().any(|s| s.label == "cache-misses"),
        "a multiplexed counter gives no value"
    );
    assert!(!stat.scores.iter().any(|s| s.label == "instructions"));
    assert!(!stat.scores.iter().any(|s| s.label == "power/energy-pkg/"));
    assert!(!stat.application_proof);
    assert!(import_perf_stat("nothing here").is_err());
    assert!(
        import_perf_stat("<not counted>,,cycles,0,0.00,,\n").is_ok_and(|s| s.scores.len() == 1)
    );
}

#[test]
fn a_perf_report_gives_sample_shares_per_symbol_and_skips_headers_and_noise() {
    let text = "# Overhead  Command  Shared Object  Symbol\n# ........\n    41.20%  game  game  [.] render_frame\n     7.05%  game  libc.so.6  [.] memcpy\n     2.00%  kworker  [kernel.kallsyms]  [k] _raw_spin_lock\n  not a row\n   150.00%  bad  bad  [.] overflow\n";
    let report = import_perf_report(text).unwrap();
    assert_eq!(value(&report, "game:game:render_frame"), 41.2);
    assert_eq!(value(&report, "game:libc.so.6:memcpy"), 7.05);
    assert_eq!(report.scores.len(), 3, "an impossible share is skipped");
    assert!(!report.application_proof);
    assert!(import_perf_report("# only a header\n").is_err());
}
