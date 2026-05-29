use super::*;

#[test]
fn foreground_source_does_not_auto_target_pid_one() {
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground_snapshot(Some(1))),
        vec![test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20, 21],
            Some(20),
            "browser",
            0.90,
        )],
    );
    snapshot
        .processes
        .insert(1, test_process(1, 0, SystemTaskClass::Service));

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn foreground_source_does_not_auto_target_systemd() {
    let mut snapshot = foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let process = snapshot.processes.get_mut(&30).unwrap();
    process.comm = "systemd".to_owned();
    process.cmdline = "/usr/lib/systemd/systemd --user".to_owned();

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn foreground_source_does_not_auto_target_compositor() {
    let mut snapshot = foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let process = snapshot.processes.get_mut(&30).unwrap();
    process.classification.class = SystemTaskClass::Compositor;
    process.classification.priority_band = PriorityBand::ForegroundLatency;

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn foreground_source_does_not_auto_target_realtime_audio_or_input() {
    let mut audio_snapshot =
        foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let audio_process = audio_snapshot.processes.get_mut(&30).unwrap();
    audio_process.classification.class = SystemTaskClass::AudioRealtime;
    audio_process.classification.priority_band = PriorityBand::CriticalRealtime;

    apply_foreground_source_mode_to_snapshot(&mut audio_snapshot, FocusSource::Foreground);

    assert!(audio_snapshot.groups.is_empty());

    let mut input_snapshot =
        foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let input_process = input_snapshot.processes.get_mut(&30).unwrap();
    input_process.classification.class = SystemTaskClass::Input;
    input_process.classification.priority_band = PriorityBand::CriticalRealtime;

    apply_foreground_source_mode_to_snapshot(&mut input_snapshot, FocusSource::Foreground);

    assert!(input_snapshot.groups.is_empty());
}

#[test]
fn foreground_source_does_not_auto_target_xwayland() {
    let mut snapshot = foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let process = snapshot.processes.get_mut(&30).unwrap();
    process.comm = "Xwayland".to_owned();
    process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn hybrid_source_unsafe_foreground_falls_back_to_existing_heuristic_groups() {
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground_snapshot(Some(30))),
        vec![test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20, 21],
            Some(20),
            "browser",
            0.90,
        )],
    );
    let process = snapshot.processes.get_mut(&30).unwrap();
    process.comm = "Xwayland".to_owned();
    process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "browser");
    assert_eq!(snapshot.groups[0].root_pids, vec![20]);
    assert_eq!(snapshot.groups[0].score, 0.90);
    assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
}
