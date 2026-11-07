use super::*;
use crate::telemetry::registration::PeerCredentials;

const MS: u64 = 1_000_000;

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
            start_ticks: 7,
        },
        primary: spec("request_p99", Aggregation::P99),
        hard_guardrails: vec![spec("error_rate", Aggregation::Mean)],
        advisory_guardrails: vec![spec("cpu_ms", Aggregation::Mean)],
    }
}

fn peer(pid: u32, uid: u32) -> PeerCredentials {
    PeerCredentials { pid, uid }
}

fn line(metric: &str, value: f64, at_ms: u64, population: u64) -> String {
    format!(
        r#"{{"metric":"{metric}","value":{value},"monotonic_ns":{},"work_unit":null,"population":{population}}}"#,
        at_ms * MS
    )
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
fn only_the_bound_process_of_the_bound_user_may_report() {
    let mut store = SampleStore::new(contract());
    let good = line("request_p99", 12.0, 1000, 20);
    assert_eq!(
        store.ingest_line(peer(42, 0), &good, 1000 * MS),
        Err(IngestDenial::WrongUser)
    );
    assert_eq!(
        store.ingest_line(peer(43, 1000), &good, 1000 * MS),
        Err(IngestDenial::NotTheBoundProcess)
    );
    assert_eq!(store.ingest_line(peer(42, 1000), &good, 1000 * MS), Ok(()));
}

#[test]
fn bad_stale_unordered_and_unknown_samples_are_refused_not_coerced() {
    let mut store = SampleStore::new(contract());
    let me = peer(42, 1000);
    assert_eq!(
        store.ingest_line(me, "not json", 0),
        Err(IngestDenial::BadJson)
    );
    assert_eq!(
        store.ingest_line(me, &"x".repeat(600), 0),
        Err(IngestDenial::LineTooLong)
    );
    assert_eq!(
        store.ingest_line(me, &line("other", 1.0, 100, 5), 100 * MS),
        Err(IngestDenial::UnknownMetric)
    );
    assert_eq!(
        store.ingest_line(me, &line("request_p99", 1.0, 100, 0), 100 * MS),
        Err(IngestDenial::EmptyPopulation)
    );
    let nan = r#"{"metric":"request_p99","value":1e999,"monotonic_ns":1,"work_unit":null,"population":5}"#;
    assert!(store.ingest_line(me, nan, 1).is_err());
    assert_eq!(
        store.ingest_line(me, &line("request_p99", 1.0, 100, 5), 100_000 * MS),
        Err(IngestDenial::TooOld)
    );
    store
        .ingest_line(me, &line("request_p99", 1.0, 200, 5), 200 * MS)
        .unwrap();
    assert_eq!(
        store.ingest_line(me, &line("request_p99", 1.0, 150, 5), 200 * MS),
        Err(IngestDenial::OutOfOrder)
    );
}

#[test]
fn a_window_needs_population_freshness_and_uses_the_declared_aggregation() {
    let mut store = SampleStore::new(contract());
    let me = peer(42, 1000);
    for (index, value) in [10.0, 20.0, 30.0, 40.0].iter().enumerate() {
        let at = 1000 + index as u64 * 100;
        store
            .ingest_line(me, &line("request_p99", *value, at, 3), at * MS)
            .unwrap();
    }
    // 12 events across the window: p99 of the four reported values.
    let window = store
        .window("request_p99", 1000 * MS, 1300 * MS, 1400 * MS)
        .unwrap();
    assert_eq!(
        (window.value, window.samples, window.population),
        (40.0, 4, 12)
    );
    // Too few events in a narrower window.
    assert!(
        store
            .window("request_p99", 1000 * MS, 1100 * MS, 1400 * MS)
            .is_none()
    );
    // Stale: the newest sample is older than max_age.
    assert!(
        store
            .window("request_p99", 1000 * MS, 1300 * MS, 100_000 * MS)
            .is_none()
    );
    assert!(store.window("unknown", 0, u64::MAX, 0).is_none());
}
