use super::*;

/// With nothing launched there is nothing to settle: the blocking stop returns at once instead of waiting out its
/// settlement window, which is why the no-driver stops in `driver_attempts.rs` cannot stall the controller's tick.
#[test]
fn a_port_that_launched_nothing_stops_at_once_even_with_a_long_settle_window() {
    let mut port = ProductionSteamPort::new(
        "/s".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        crate::autotune::research::tests::intent().steam_client,
        vec![],
        Probe,
    )
    .with_settle_window(Duration::from_secs(120));
    let started = std::time::Instant::now();
    assert!(port.stop().is_ok());
    assert!(started.elapsed() < Duration::from_secs(2));
}
