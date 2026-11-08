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
