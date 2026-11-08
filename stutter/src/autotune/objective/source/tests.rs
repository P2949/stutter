use super::*;
use crate::telemetry::registration::PeerCredentials;

const MS: u64 = 1_000_000;
const TICKS: u64 = 7;

fn spec(id: &str, aggregation: Aggregation) -> MetricSpec {
    MetricSpec {
        id: id.to_owned(),
        unit: "ms".to_owned(),
        class: MetricClass::Latency,
        direction: Direction::LowerIsBetter,
        aggregation,
        max_age_ms: 5_000,
        min_population: 10,
        desired_value: None,
        meaningful_effect_percent: 10.0,
        equivalent_within_percent: 3.0,
    }
}

fn contract() -> KpiContract {
    KpiContract {
        binding: SourceBinding {
            uid: 1000,
            pid: 42,
            start_ticks: TICKS,
        },
        primary: spec("request_p99", Aggregation::P99),
        hard_guardrails: vec![spec("error_rate", Aggregation::Mean)],
        advisory_guardrails: vec![spec("cpu_ms", Aggregation::Mean)],
    }
}

fn peer(pid: u32, uid: u32) -> PeerCredentials {
    PeerCredentials { pid, uid }
}

fn line(metric: &str, value: f64, at_ms: u64, population: u64, kind: &str) -> String {
    format!(
        r#"{{"metric":"{metric}","value":{value},"monotonic_ns":{},"work_unit":null,"population":{population},"kind":"{kind}"}}"#,
        at_ms * MS
    )
}

fn feed(store: &mut SampleStore, metric: &str, values: &[(f64, u64, &str)], start_ms: u64) {
    for (index, (value, population, kind)) in values.iter().enumerate() {
        let at = start_ms + index as u64 * 100;
        store
            .ingest_line(
                peer(42, 1000),
                TICKS,
                &line(metric, *value, at, *population, kind),
                at * MS,
            )
            .unwrap();
    }
}

#[test]
fn a_contract_needs_one_primary_distinct_metrics_and_a_hard_guardrail() {
    assert_eq!(contract().validate(), Ok(()));
    assert!(!contract().authorizes_mutation());
    let mut duplicate = contract();
    duplicate.advisory_guardrails[0].id = "request_p99".into();
    assert!(matches!(
        duplicate.validate(),
        Err(ContractError::DuplicateMetric(_))
    ));
    let mut none = contract();
    none.hard_guardrails.clear();
    assert_eq!(none.validate(), Err(ContractError::NoHardGuardrail));
    let mut vague = contract();
    vague.primary.meaningful_effect_percent = 2.0;
    assert!(matches!(
        vague.validate(),
        Err(ContractError::BadSpec(_, _))
    ));
    let mut nan = contract();
    nan.primary.desired_value = Some(f64::NAN);
    assert!(nan.validate().is_err());
}

#[test]
fn only_the_bound_live_process_of_the_bound_user_may_report() {
    let mut store = SampleStore::new(contract());
    let good = line("request_p99", 12.0, 1000, 20, "summary");
    assert_eq!(
        store.ingest_line(peer(42, 0), TICKS, &good, 1000 * MS),
        Err(IngestDenial::WrongUser)
    );
    assert_eq!(
        store.ingest_line(peer(43, 1000), TICKS, &good, 1000 * MS),
        Err(IngestDenial::NotTheBoundProcess)
    );
    // The same pid and uid but a different start time is a reused pid.
    assert_eq!(
        store.ingest_line(peer(42, 1000), TICKS + 1, &good, 1000 * MS),
        Err(IngestDenial::PeerStartTimeMismatch)
    );
    assert_eq!(
        store.ingest_line(peer(42, 1000), TICKS, &good, 1000 * MS),
        Ok(())
    );
}

#[test]
fn bad_stale_future_replayed_and_unknown_samples_are_refused_not_coerced() {
    let mut store = SampleStore::new(contract());
    let ingest = |store: &mut SampleStore, text: &str, now: u64| {
        store.ingest_line(peer(42, 1000), TICKS, text, now)
    };
    assert_eq!(
        ingest(&mut store, "not json", 0),
        Err(IngestDenial::BadJson)
    );
    assert_eq!(
        ingest(&mut store, &"x".repeat(600), 0),
        Err(IngestDenial::LineTooLong)
    );
    assert_eq!(
        ingest(&mut store, &line("other", 1.0, 100, 5, "summary"), 100 * MS),
        Err(IngestDenial::UnknownMetric)
    );
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 100, 0, "summary"),
            100 * MS
        ),
        Err(IngestDenial::EmptyPopulation)
    );
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", -1.0, 100, 5, "summary"),
            100 * MS
        ),
        Err(IngestDenial::NegativeValue)
    );
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 100, 3, "raw_event"),
            100 * MS
        ),
        Err(IngestDenial::RawEventPopulation)
    );
    let huge = r#"{"metric":"request_p99","value":1e999,"monotonic_ns":1,"work_unit":null,"population":5}"#;
    assert!(ingest(&mut store, huge, 1).is_err());
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 100, 5, "summary"),
            100_000 * MS
        ),
        Err(IngestDenial::TooOld)
    );
    // A timestamp far ahead of the local clock is forged.
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 10_000, 5, "summary"),
            100 * MS
        ),
        Err(IngestDenial::FutureTimestamp)
    );
    ingest(
        &mut store,
        &line("request_p99", 1.0, 200, 5, "summary"),
        200 * MS,
    )
    .unwrap();
    // Out of order and an exact duplicate are both replays.
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 150, 5, "summary"),
            200 * MS
        ),
        Err(IngestDenial::NotAfterPrevious)
    );
    assert_eq!(
        ingest(
            &mut store,
            &line("request_p99", 1.0, 200, 5, "summary"),
            200 * MS
        ),
        Err(IngestDenial::NotAfterPrevious)
    );
}

#[test]
fn a_mean_of_summaries_is_weighted_by_the_events_each_one_stands_for() {
    let mut store = SampleStore::new(contract());
    // 90 fast events averaging 10 and 10 slow events averaging 110: the true mean is 20, not 60.
    feed(
        &mut store,
        "error_rate",
        &[(10.0, 90, "summary"), (110.0, 10, "summary")],
        1000,
    );
    let window = store
        .window("error_rate", 1000 * MS, 1100 * MS, 1200 * MS)
        .unwrap();
    assert!((window.value - 20.0).abs() < 1e-9, "{window:?}");
    assert_eq!(window.semantics, Semantics::Direct);
    assert_eq!(window.population, 100);
}

#[test]
fn a_percentile_of_batch_percentiles_is_never_a_direct_percentile() {
    let mut store = SampleStore::new(contract());
    feed(
        &mut store,
        "request_p99",
        &[
            (10.0, 5, "summary"),
            (20.0, 5, "summary"),
            (40.0, 5, "summary"),
        ],
        1000,
    );
    let summaries = store
        .window("request_p99", 1000 * MS, 1200 * MS, 1300 * MS)
        .unwrap();
    assert_eq!(summaries.value, 40.0);
    assert_eq!(summaries.semantics, Semantics::QuantileOfSummaries);

    let mut raw = SampleStore::new(contract());
    let events: Vec<(f64, u64, &str)> = (1..=20).map(|v| (f64::from(v), 1, "raw_event")).collect();
    feed(&mut raw, "request_p99", &events, 1000);
    let window = raw
        .window("request_p99", 1000 * MS, 3000 * MS, 3100 * MS)
        .unwrap();
    assert_eq!(window.value, 20.0);
    assert_eq!(window.semantics, Semantics::Direct);

    // One summary among raw events downgrades the whole window.
    let mut mixed = SampleStore::new(contract());
    let mut events: Vec<(f64, u64, &str)> =
        (1..=12).map(|v| (f64::from(v), 1, "raw_event")).collect();
    events.push((50.0, 5, "summary"));
    feed(&mut mixed, "request_p99", &events, 1000);
    let window = mixed
        .window("request_p99", 1000 * MS, 3000 * MS, 3100 * MS)
        .unwrap();
    assert_eq!(window.semantics, Semantics::QuantileOfSummaries);
}

#[test]
fn a_window_needs_population_and_freshness() {
    let mut store = SampleStore::new(contract());
    feed(
        &mut store,
        "request_p99",
        &[
            (10.0, 3, "summary"),
            (20.0, 3, "summary"),
            (30.0, 3, "summary"),
            (40.0, 3, "summary"),
        ],
        1000,
    );
    assert!(
        store
            .window("request_p99", 1000 * MS, 1300 * MS, 1400 * MS)
            .is_some()
    );
    assert!(
        store
            .window("request_p99", 1000 * MS, 1100 * MS, 1400 * MS)
            .is_none(),
        "too few events"
    );
    assert!(
        store
            .window("request_p99", 1000 * MS, 1300 * MS, 100_000 * MS)
            .is_none(),
        "stale"
    );
    assert!(store.window("unknown", 0, u64::MAX, 0).is_none());
}

#[test]
fn a_capacity_eviction_is_reported_as_an_incomplete_window() {
    let mut small_population = contract();
    small_population.primary.min_population = 1;
    let mut store = SampleStore::new(small_population);
    // 4100 raw events: the first 4 fall out of the 4096-sample ring.
    for index in 0..4100_u64 {
        let at = 1000 + index;
        store
            .ingest_line(
                peer(42, 1000),
                TICKS,
                &line("request_p99", 5.0, at, 1, "raw_event"),
                at * MS,
            )
            .unwrap();
    }
    let now = 1000 + 4100;
    let whole = store
        .window("request_p99", 1000 * MS, now * MS, now * MS)
        .unwrap();
    assert!(!whole.complete, "the window reaches into evicted samples");
    assert_eq!(whole.samples, 4096);
    let recent = store
        .window("request_p99", 1100 * MS, now * MS, now * MS)
        .unwrap();
    assert!(recent.complete);
}

mod socket {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        path::Path,
        time::Instant,
    };

    use super::super::*;
    use crate::telemetry::proc_view::live_process;

    fn contract_for_this_process() -> KpiContract {
        let pid = std::process::id();
        let live = live_process(Path::new("/proc"), pid).unwrap();
        KpiContract {
            binding: SourceBinding {
                uid: crate::syscall::geteuid(),
                pid,
                start_ticks: live.start_ticks,
            },
            primary: super::spec("request_p99", Aggregation::P99),
            hard_guardrails: vec![super::spec("error_rate", Aggregation::Mean)],
            advisory_guardrails: vec![],
        }
    }

    fn line(metric: &str, at_ms: u64) -> String {
        format!(
            r#"{{"metric":"{metric}","value":5.0,"monotonic_ns":{},"work_unit":null,"population":20,"kind":"summary"}}"#,
            at_ms * 1_000_000
        )
    }

    #[test]
    fn samples_over_a_socket_are_authenticated_by_the_kernel_and_answered_one_by_one() {
        let (client, server) = UnixStream::pair().unwrap();
        let mut store = SampleStore::new(contract_for_this_process());
        let began = Instant::now();
        let now = move || began.elapsed().as_nanos() as u64 + 10_000_000_000;
        let base_ms = 10_000;
        let mut writer = &client;
        writeln!(writer, "{}", line("request_p99", base_ms)).unwrap();
        writeln!(writer, "{}", line("request_p99", base_ms)).unwrap(); // a replay
        writeln!(writer, "{}", line("nonsense", base_ms + 5)).unwrap();
        writeln!(writer, "not json").unwrap();
        writeln!(writer, "{}", line("request_p99", base_ms + 10)).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let accepted = serve_connection(&mut store, server, Path::new("/proc"), now).unwrap();
        assert_eq!(accepted, 2);
        let replies: Vec<String> = BufReader::new(&client)
            .lines()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            replies,
            [
                "ok",
                "denied not_after_previous",
                "denied unknown_metric",
                "denied bad_json",
                "ok"
            ]
        );
    }

    #[test]
    fn a_peer_that_is_not_the_bound_process_gets_only_denials() {
        let (client, server) = UnixStream::pair().unwrap();
        // Bound to a different process than this test's.
        let mut contract = contract_for_this_process();
        contract.binding.pid = std::process::id().wrapping_add(1);
        let mut store = SampleStore::new(contract);
        let mut writer = &client;
        writeln!(writer, "{}", line("request_p99", 10_000)).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let accepted =
            serve_connection(&mut store, server, Path::new("/proc"), || 10_000_000_000).unwrap();
        assert_eq!(accepted, 0);
        let reply = BufReader::new(&client).lines().next().unwrap().unwrap();
        assert_eq!(reply, "denied not_the_bound_process");
    }

    #[test]
    fn an_oversize_line_is_denied_and_a_connection_is_bounded() {
        let (client, server) = UnixStream::pair().unwrap();
        let mut store = SampleStore::new(contract_for_this_process());
        let mut writer = &client;
        writeln!(writer, "{}", "x".repeat(600)).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        serve_connection(&mut store, server, Path::new("/proc"), || 0).unwrap();
        let reply = BufReader::new(&client).lines().next().unwrap().unwrap();
        assert_eq!(reply, "denied line_too_long");
        assert_eq!(
            denial_code(IngestDenial::FutureTimestamp),
            "future_timestamp"
        );
    }
}

mod observer {
    use std::{io::Write, os::unix::net::UnixStream, path::Path};

    use super::*;
    use crate::{
        autotune::objective::source::serve_observer,
        telemetry::{proc_view::live_process, registration::bind},
    };

    #[test]
    fn the_observer_authenticates_the_peer_and_prints_a_summary_per_metric() {
        let dir = std::env::temp_dir().join(format!("stutter-kpi-obs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let uid = crate::syscall::geteuid();
        let listener = bind(&dir, "kpi.sock", uid).unwrap();
        let pid = std::process::id();
        let live = live_process(Path::new("/proc"), pid).unwrap();
        let contract = KpiContract {
            binding: SourceBinding {
                uid,
                pid,
                start_ticks: live.start_ticks,
            },
            primary: spec("request_p99", Aggregation::Mean),
            hard_guardrails: vec![spec("error_rate", Aggregation::Mean)],
            advisory_guardrails: vec![],
        };
        let socket = dir.join("kpi.sock");
        let client = std::thread::spawn(move || {
            let mut stream = UnixStream::connect(socket).unwrap();
            let at = crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap() - 1_000_000;
            writeln!(stream, r#"{{"metric":"request_p99","value":7.5,"monotonic_ns":{at},"work_unit":null,"population":20,"kind":"summary"}}"#).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            let mut reply = String::new();
            std::io::Read::read_to_string(&mut stream, &mut reply).unwrap();
            reply
        });
        let mut out = Vec::new();
        let handled = serve_observer(
            contract,
            &listener,
            Path::new("/proc"),
            60_000_000_000,
            Some(1),
            &mut out,
        )
        .unwrap();
        assert_eq!(handled, 1);
        let reply = client.join().unwrap();
        assert_eq!(reply, "ok\n");
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one line per contract metric: {text}");
        assert!(
            lines[0].contains("\"metric\":\"request_p99\"") && lines[0].contains("\"value\":7.5"),
            "{text}"
        );
        assert!(
            lines[1].contains("\"metric\":\"error_rate\"") && lines[1].contains("\"value\":null")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
