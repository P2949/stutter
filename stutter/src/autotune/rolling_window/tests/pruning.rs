use super::*;

#[test]
fn default_window_is_thirty_seconds() {
    let window = RollingWindow::default();

    assert_eq!(window.duration(), Duration::from_secs(30));
    assert!(window.is_empty());
}

#[test]
fn push_interval_prunes_old_intervals_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_interval(interval(1000, 10));
    window.push_interval(interval(2500, 20));
    window.push_interval(interval(3501, 30));

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2500, 3501]
    );
    assert_eq!(window.scored_samples(), 50);
}

#[test]
fn push_frame_prunes_old_frames_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    window.push_frame(frame(1000, 16.0));
    window.push_frame(frame(1500, 17.0));
    window.push_frame(frame(2101, 18.0));

    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500, 2101]
    );
}

#[test]
fn push_frame_drops_invalid_frametimes_and_counts_them() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1050, 16.0));
    window.push_frame(frame(1100, f64::NAN));
    window.push_frame(frame(1200, -1.0));
    window.push_frame(frame(1300, 20.0));

    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.frametime_ms)
            .collect::<Vec<_>>(),
        vec![16.0, 20.0]
    );
    assert_eq!(window.frame_max_ms(), 20.0);
    assert_eq!(window.frame_p99_ms(), 20.0);
}

#[test]
fn dropped_invalid_frametime_still_advances_window_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    window.push_frame(frame(1000, 16.0));
    window.push_frame(frame(2501, 0.0));

    assert!(window.frames.is_empty());
    assert_eq!(window.dropped_invalid_frame_count(), 1);
}

#[test]
fn push_diagnosis_prunes_old_diagnoses_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(3));

    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(3000, StutterCause::GpuBoundCandidate));
    window.push_diagnosis(diagnosis(4501, StutterCause::GameThreadSchedulerDelay));

    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![3000, 4501]
    );
}

#[test]
fn prune_to_prunes_all_streams_using_same_cutoff() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.intervals.push_back(interval(1000, 10));
    window.intervals.push_back(interval(3000, 20));
    window.frames.push_back(frame(999, 16.0));
    window.frames.push_back(frame(2500, 22.0));
    window
        .diagnoses
        .push_back(diagnosis(1500, StutterCause::Unknown));
    window
        .diagnoses
        .push_back(diagnosis(3200, StutterCause::CpuPressureCandidate));

    window.prune_to(3500);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![3000]
    );
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2500]
    );
    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500, 3200]
    );
}

#[test]
fn retain_latest_window_uses_latest_event_across_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.intervals.push_back(interval(1000, 10));
    window.frames.push_back(frame(1500, 17.0));
    window
        .diagnoses
        .push_back(diagnosis(2300, StutterCause::CpuPressureCandidate));

    window.retain_latest_window();

    assert!(window.intervals.is_empty());
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500]
    );
    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2300]
    );
}

#[test]
fn push_intervals_prunes_once_using_latest_inserted_elapsed_ms() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_intervals(vec![
        interval(1000, 1),
        interval(2000, 2),
        interval(3501, 3),
    ]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2000, 3501]
    );
    assert_eq!(window.scored_samples(), 5);
}

#[test]
fn push_intervals_sorts_out_of_order_batches_before_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_intervals(vec![
        interval(4501, 4),
        interval(1000, 1),
        interval(2500, 2),
    ]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![4501]
    );
    assert_eq!(window.scored_samples(), 4);
}

#[test]
fn push_intervals_prunes_old_records_even_when_batch_arrives_after_newer_record() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.push_interval(interval(6000, 6));

    window.push_intervals(vec![interval(2500, 1), interval(4500, 4)]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![4500, 6000]
    );
    assert_eq!(window.latest_elapsed_ms(), Some(6000));
    assert_eq!(window.scored_samples(), 10);
}

#[test]
fn push_intervals_preserves_same_tick_batch_order() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    let mut first = interval(1000, 1);
    first.task = 1;
    let mut second = interval(1000, 2);
    second.task = 2;

    window.push_intervals(vec![first, second]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.task)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn push_intervals_empty_batch_is_noop() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.push_interval(interval(1000, 1));

    window.push_intervals(Vec::new());

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1000]
    );
    assert_eq!(window.scored_samples(), 1);
}

#[test]
fn latest_elapsed_ms_uses_max_across_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.intervals.push_back(interval(1000, 10));
    window.frames.push_back(frame(4000, 16.0));
    window
        .diagnoses
        .push_back(diagnosis(2500, StutterCause::Unknown));

    assert_eq!(window.latest_elapsed_ms(), Some(4000));
}

#[test]
fn clear_removes_all_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_interval(interval(1000, 10));
    window.push_frame(frame(1000, 16.0));
    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));

    window.clear();

    assert!(window.is_empty());
    assert_eq!(window.total_event_count(), 0);
}
