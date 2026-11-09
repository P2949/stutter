use super::profiles::*;

fn value(scores: &super::scores::ImportedScores, label: &str) -> Option<f64> {
    scores
        .scores
        .iter()
        .find(|s| s.label == label)
        .map(|s| s.value)
}

#[test]
fn perf_script_rows_become_per_event_samples_periods_and_per_command_shares() {
    let text = "      game  4242 [003] 12345.678901:     250000 cycles:  ffffffff810 do_work+0x1 ([kernel.kallsyms])\n      game  4242 [003] 12345.679901:     250000 cycles:  7fff1 render+0x10 (/usr/lib/libgame.so)\n  kworker/u8:1  91 [001] 12345.680000:     100000 cycles:  ffffffff811 worker+0x2 ([kernel.kallsyms])\nthis row is garbage\n      game  4242 [003] 12345.690000:        -5 cycles: bad\n";
    let imported = import_perf_script(text).unwrap();
    assert!(!imported.application_proof);
    assert_eq!(value(&imported, "cycles.samples"), Some(3.0));
    assert_eq!(value(&imported, "cycles.period"), Some(600_000.0));
    let game = value(&imported, "game.share").unwrap();
    assert!((game - 66.666_666).abs() < 1e-3, "{game}");
    assert!(imported.scores.iter().all(|s| s.higher_is_better.is_none()));
    assert!(import_perf_script("").is_err());
    assert!(import_perf_script("nothing parseable here\n").is_err());
}

#[test]
fn profile_tables_use_the_first_column_as_the_row_and_skip_non_numeric_cells() {
    let vtune =
        "Function,CPU Time,Instructions Retired\nrender_frame,12.5,\"1,000\"\nlock_wait,3.25,400\n";
    let imported = import_profile_table(ProfileTool::Vtune, vtune).unwrap();
    assert_eq!(value(&imported, "render_frame.CPU Time"), Some(12.5));
    assert_eq!(
        value(&imported, "lock_wait.Instructions Retired"),
        Some(400.0)
    );
    assert_eq!(
        value(&imported, "render_frame.Instructions Retired"),
        None,
        "a thousands-separated cell is not guessed at"
    );
    let likwid = "Metric,Core 0,Core 1\nRuntime (RDTSC) [s],1.5,1.6\nCPI,0.9,1.1\n";
    let imported = import_profile_table(ProfileTool::Likwid, likwid).unwrap();
    assert_eq!(value(&imported, "CPI.Core 1"), Some(1.1));
    assert_eq!(imported.tool, "likwid");
    let uprof = "FUNCTION,TIMER SAMPLES,CPU_CLK_UNHALTED\nmain,10,200\n";
    assert_eq!(
        value(
            &import_profile_table(ProfileTool::Uprof, uprof).unwrap(),
            "main.TIMER SAMPLES"
        ),
        Some(10.0)
    );
    assert!(import_profile_table(ProfileTool::Vtune, "").is_err());
    assert!(import_profile_table(ProfileTool::Vtune, "just one line\n").is_err());
    assert!(import_profile_table(ProfileTool::Likwid, "a,b\nonly,words\n").is_err());
}
