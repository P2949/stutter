use super::*;

fn epoch(unix: u64, events: u64, max_ns: u64, bucket: usize) -> Epoch {
    let mut counts = vec![0; LatencyHistogram::new().snapshot().len()];
    counts[bucket] = events;
    Epoch {
        unix,
        events,
        max_ns,
        counts,
    }
}

#[test]
fn a_population_is_the_epochs_inside_the_half_open_interval_in_the_canonical_layout() {
    let epochs = [
        epoch(99, 10, 500, 0),
        epoch(100, 20, 900, 1),
        epoch(101, 30, 700, 1),
        epoch(110, 40, 100, 2),
    ];
    let (buckets, max_ns, events) = population(&epochs, 100, 110).unwrap();
    assert_eq!((events, max_ns), (50, 900));
    assert_eq!(buckets[1].count, 50);
    assert_eq!(
        buckets.iter().map(|b| b.count).sum::<u64>(),
        50,
        "the epochs before and at the end of the interval are not counted"
    );
    // An epoch in another layout is refused, not reinterpreted.
    let mut odd = epoch(100, 1, 1, 0);
    odd.counts.pop();
    assert_eq!(population(&[odd], 0, 1_000), None);
}

#[test]
fn the_off_attempt_is_read_over_the_same_offsets_from_its_own_start() {
    let receipt = serde_json::json!({ "applied_unix": 1_300, "settled_unix": 1_900 });
    let treated = treated_interval(&receipt).unwrap();
    assert_eq!(treated, (1_300, 1_900));
    // The On attempt started at 1_000 (the action 300 s in); the Off attempt started at 5_000.
    assert_eq!(
        pair_ranges(treated, 1_000, 5_000),
        ((1_300, 1_900), (5_300, 5_900))
    );
    // A receipt that never shows a held interval has none.
    assert_eq!(
        treated_interval(&serde_json::json!({ "applied_unix": 5, "settled_unix": 5 })),
        None
    );
    assert_eq!(treated_interval(&serde_json::json!({})), None);
}
