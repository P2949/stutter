use super::*;

#[test]
fn untimestamped_irq_events_are_timestamped_at_ingestion() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.push_interval(interval(5_000, 10));
    let mut event = irq_event(0, 3_000_000);
    event.elapsed_ms = None;

    window.push_irq_event(event);

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|event| event.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![Some(5_000)]
    );
}

#[test]
fn untimestamped_irq_events_without_time_anchor_are_dropped() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    for _ in 0..256 {
        let mut event = irq_event(0, 3_000_000);
        event.elapsed_ms = None;
        window.push_irq_event(event);
    }

    assert!(window.irq_events.is_empty());
    assert_eq!(window.latest_elapsed_ms(), None);
}

#[test]
fn later_timestamped_irq_events_prune_ingestion_timestamped_events() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.push_interval(interval(5_000, 10));
    let mut untimestamped = irq_event(0, 3_000_000);
    untimestamped.elapsed_ms = None;
    window.push_irq_event(untimestamped);

    window.push_irq_event(irq_event(6_500, 1_000_000));

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|event| event.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![Some(6_500)]
    );
}

#[test]
fn prune_to_drops_legacy_untimestamped_irq_events() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    let mut event = irq_event(0, 3_000_000);
    event.elapsed_ms = None;
    window.irq_events.push_back(event);

    window.prune_to(2_000);

    assert!(window.irq_events.is_empty());
}
