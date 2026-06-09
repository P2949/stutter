use super::*;

#[test]
fn push_frames_accepts_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(20_000, 16.0));
    window.push_frame(frame(15_000, 16.0));
    window.push_frame(frame(26_000, 16.0));

    assert_eq!(
        window
            .frames
            .iter()
            .map(|f| f.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.frames, |f| f.elapsed_ms);
}

#[test]
fn push_diagnoses_accepts_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_diagnosis(diagnosis(20_000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(15_000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(26_000, StutterCause::Unknown));

    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|d| d.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.diagnoses, |d| d.elapsed_ms);
}

#[test]
fn push_irq_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_irq_event(irq_event(20_000, 100));
    window.push_irq_event(irq_event(15_000, 100));
    window.push_irq_event(irq_event(26_000, 100));

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|e| e.elapsed_ms.unwrap())
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.irq_events, |e| e.elapsed_ms.unwrap_or(0));
}

#[test]
fn push_block_io_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_block_io_event(block_io_event(20_000, 100));
    window.push_block_io_event(block_io_event(15_000, 100));
    window.push_block_io_event(block_io_event(26_000, 100));

    assert_eq!(
        window
            .block_io_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.block_io_events, |e| e.elapsed_ms);
}

#[test]
fn push_gpu_samples_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_gpu_sample(gpu_sample(20_000, 50));
    window.push_gpu_sample(gpu_sample(15_000, 50));
    window.push_gpu_sample(gpu_sample(26_000, 50));

    assert_eq!(
        window
            .gpu_samples
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.gpu_samples, |e| e.elapsed_ms);
}

#[test]
fn push_cpu_freq_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 20_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 15_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 26_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });

    assert_eq!(
        window
            .cpu_freq_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.cpu_freq_events, |e| e.elapsed_ms);
}

#[test]
fn push_foreground_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 20_000,
        ..Default::default()
    });
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 15_000,
        ..Default::default()
    });
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 26_000,
        ..Default::default()
    });

    assert_eq!(
        window
            .foreground_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.foreground_events, |e| e.elapsed_ms);
}

#[test]
fn push_sorted_by_elapsed_preserves_same_tick_insertion_order() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    let mut first = frame(10_000, 16.0);
    first.frametime_ms = 10.0;
    let mut second = frame(10_000, 16.0);
    second.frametime_ms = 20.0;

    window.push_frame(first);
    window.push_frame(second);

    assert_eq!(
        window
            .frames
            .iter()
            .map(|f| f.frametime_ms)
            .collect::<Vec<_>>(),
        vec![10.0, 20.0]
    );
}
