use super::scores::*;

const SYSBENCH: &str = "sysbench 1.0.20 (using system LuaJIT 2.1.0-beta3)\n\nCPU speed:\n    events per second:  1234.56\n\nGeneral statistics:\n    total time:                          10.0003s\n\nLatency (ms):\n         min:                                    0.80\n         avg:                                    0.81\n         95th percentile:                        0.83\n";
const STRESS: &str = "stress-ng: info:  [12] setting to a 10 secs run per stressor\nstress-ng: metrc: [12] stressor       bogo ops real time  usr time  sys time   bogo ops/s     bogo ops/s\nstress-ng: metrc: [12]                           (secs)    (secs)    (secs)   (real time) (usr+sys time)\nstress-ng: metrc: [12] cpu               52000     10.00     39.90      0.01      5200.00        1303.00\nstress-ng: metrc: [12] matrix            9000      10.00     39.90      0.01       900.00         225.00\n";
const BENCHSTAT: &str = "name        old time/op  new time/op  delta\nParse-8     1.00ms ± 1%  0.90ms ± 2%  -10.00%  (p=0.000 n=10+10)\nRender-8    2.00ms ± 1%  2.00ms ± 1%     ~     (p=0.500 n=10+10)\n";
const PHORONIX: &str = "<?xml version=\"1.0\"?>\n<PhoronixTestSuite><Result><Title>Zstd</Title><Scale>MB/s</Scale><Proportion>HIB</Proportion><Data><Entry><Identifier>a</Identifier><Value>1500.5</Value></Entry></Data></Result></PhoronixTestSuite>";

#[test]
fn each_tool_parses_into_typed_scores_and_none_is_application_proof() {
    let sysbench = import_sysbench(SYSBENCH).unwrap();
    assert_eq!(sysbench.scores.len(), 4);
    assert_eq!(sysbench.scores[0].value, 1234.56);
    assert!(!sysbench.application_proof);

    let stress = import_stress_ng(STRESS).unwrap();
    assert_eq!(stress.scores.len(), 2);
    assert_eq!(stress.scores[0].label, "cpu");
    assert_eq!(stress.scores[0].value, 5200.0);
    assert!(!stress.application_proof);

    let rows = import_benchstat(BENCHSTAT).unwrap();
    assert_eq!(rows.len(), 2);
    assert!((rows[0].old_seconds - 0.001).abs() < 1e-12);
    assert_eq!(rows[0].delta_percent, Some(-10.0));
    assert_eq!(rows[0].p_value, Some(0.0));
    assert_eq!(
        rows[1].delta_percent, None,
        "`~` is no significant difference"
    );

    let phoronix = import_phoronix(PHORONIX).unwrap();
    assert_eq!(phoronix.scores[0].value, 1500.5);
    assert_eq!(phoronix.scores[0].higher_is_better, Some(true));
    assert!(!phoronix.application_proof);
}

#[test]
fn malformed_empty_or_foreign_input_is_an_error_never_a_zero() {
    assert!(import_sysbench("hello").is_err());
    assert!(import_sysbench("sysbench 1.0\nevents per second: abc\n").is_err());
    assert!(import_sysbench("sysbench 1.0\n").is_err());
    assert!(import_stress_ng("").is_err());
    assert!(import_stress_ng("stress-ng: metrc: [1] stressor bogo ops real\nstress-ng: metrc: [1] cpu 1 0.00 0 0 5 5\n").is_err());
    assert!(import_benchstat("nothing").is_err());
    assert!(import_benchstat("name old time/op new time/op delta\nX 1parsec  2s  ~\n").is_err());
    assert!(import_phoronix("<xml/>").is_err());
    assert!(import_phoronix("<PhoronixTestSuite><Result><Title>t</Title><Value>NaN</Value></Result></PhoronixTestSuite>").is_err());
}

#[test]
fn a_prometheus_snapshot_is_bounded_read_only_and_drops_stale_or_non_finite_samples() {
    let text = "# HELP up x\n# TYPE up gauge\nup{job=\"app\"} 1\nhttp_requests_total{code=\"200\"} 42 9000\nold_metric 5 1000\nbad NaN\ninf_metric +Inf\nplain 7\n";
    let snapshot = import_prometheus(text, 10_000, 5_000).unwrap();
    let labels: Vec<&str> = snapshot.scores.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "up{job=\"app\"}",
            "http_requests_total{code=\"200\"}",
            "plain"
        ]
    );
    assert!(!snapshot.application_proof);
    let many = format!(
        "m{{{}}} 1\n",
        (0..9)
            .map(|i| format!("l{i}=\"v\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(import_prometheus(&many, 0, 1).is_err(), "too many labels");
    let long = format!("m{{l=\"{}\"}} 1\n", "x".repeat(200));
    assert!(import_prometheus(&long, 0, 1).is_err(), "label too long");
    assert!(
        import_prometheus("m{l=\"x\" 1\n", 0, 1).is_err(),
        "unclosed label set"
    );
    assert!(import_prometheus("# only comments\n", 0, 1).is_err());
}

#[test]
fn a_load_comparison_needs_matched_offered_load_and_never_rewards_more_errors() {
    let base = LoadRun {
        offered_per_second: 1000.0,
        completed: 9_900,
        errors: 100,
        p99_ms: 50.0,
    };
    let faster = LoadRun {
        p99_ms: 35.0,
        ..base
    };
    assert_eq!(
        compare_load(base, faster, 5.0, 10.0).unwrap(),
        LoadVerdict::CandidateFaster
    );
    let slower = LoadRun {
        p99_ms: 70.0,
        ..base
    };
    assert_eq!(
        compare_load(base, slower, 5.0, 10.0).unwrap(),
        LoadVerdict::CandidateSlower
    );
    assert_eq!(
        compare_load(
            base,
            LoadRun {
                p99_ms: 49.0,
                ..base
            },
            5.0,
            10.0
        )
        .unwrap(),
        LoadVerdict::Equivalent
    );
    let throttled = LoadRun {
        offered_per_second: 500.0,
        p99_ms: 10.0,
        ..base
    };
    assert_eq!(
        compare_load(base, throttled, 5.0, 10.0).unwrap(),
        LoadVerdict::OfferedLoadDiffers
    );
    let erroring = LoadRun {
        completed: 9_000,
        errors: 1_000,
        p99_ms: 10.0,
        ..base
    };
    assert_eq!(
        compare_load(base, erroring, 5.0, 10.0).unwrap(),
        LoadVerdict::MoreErrors
    );
    assert!(
        compare_load(
            LoadRun {
                p99_ms: f64::NAN,
                ..base
            },
            base,
            5.0,
            10.0
        )
        .is_err()
    );
    assert!(
        compare_load(
            base,
            LoadRun {
                completed: 0,
                ..base
            },
            5.0,
            10.0
        )
        .is_err()
    );
}
