use super::*;
use crate::ebpf_loader::DropCountersSnapshot;

fn lost(events: u64) -> DropCountersSnapshot {
    DropCountersSnapshot {
        ringbuf_reserve_failed: events,
        ..Default::default()
    }
}

fn lossy(elapsed_ms: u64, task: u32, events: u64) -> IntervalRecord {
    IntervalRecord {
        task,
        drop_counters: lost(events),
        ..interval(elapsed_ms, 3)
    }
}

#[test]
fn a_cumulative_counter_that_stops_moving_loses_nothing_after_the_first_interval() {
    let mut previous = DropCountersSnapshot::default();
    let mut losses = Vec::new();
    for cumulative in [0, 1, 1, 1] {
        let now = lost(cumulative);
        losses.push(now.delta_since(&previous).total());
        previous = now;
    }
    assert_eq!(losses, vec![0, 1, 0, 0]);
}

#[test]
fn a_counter_that_restarts_after_a_reload_counts_from_zero() {
    assert_eq!(lost(2).delta_since(&lost(9)).total(), 2);
    assert_eq!(lost(11).delta_since(&lost(9)).total(), 2);
}

#[test]
fn the_window_counts_each_interval_once_however_many_tasks_it_scored() {
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_intervals((1..=40).map(|task| lossy(1000, task, 1)));
    window.push_intervals((1..=40).map(|task| lossy(2000, task, 0)));
    assert_eq!(window.drop_total(), 1);
}

#[test]
fn an_old_loss_does_not_poison_a_window_that_no_longer_holds_it() {
    let mut window = RollingWindow::new(Duration::from_secs(5));
    window.push_intervals([lossy(1000, 1, 4)]);
    for second in 2..=12 {
        window.push_intervals([lossy(second * 1000, 1, 0)]);
    }
    assert_eq!(window.drop_total(), 0);
}

#[test]
fn a_new_loss_inside_the_window_is_counted_and_fails_the_zero_loss_policy() {
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_intervals([lossy(1000, 1, 0), lossy(2000, 1, 2)]);
    assert_eq!(window.drop_total(), 2);
}
