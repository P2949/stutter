//! Safety warning focus tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{
        test_support::{group_test_process as test_process, group_test_snapshot as test_snapshot},
        *,
    };

    #[test]
    fn safety_warnings_report_critical_realtime_and_compositor_members() {
        let audio = test_process(
            700,
            1,
            "pipewire",
            SystemTaskClass::AudioRealtime,
            PriorityBand::CriticalRealtime,
            5,
        );
        let compositor = test_process(
            701,
            1,
            "sway",
            SystemTaskClass::Compositor,
            PriorityBand::ForegroundLatency,
            10,
        );

        let snapshot = test_snapshot(vec![audio, compositor]);
        let group = FocusGroup {
            kind: FocusGroupKind::Desktop,
            root_pids: vec![700],
            member_pids: vec![700, 701],
            primary_pid: Some(701),
            display_name: "Desktop".to_owned(),
            score: 0.75,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.80,
            priority_band: PriorityBand::ForegroundLatency,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::CriticalRealtimePresent { pid: 700, comm } if comm == "pipewire"
        )));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::CompositorInFocusGroup { pid: 701, comm } if comm == "sway"
        )));
    }

    #[test]
    fn safety_warnings_report_unknown_active_foreground_like_process() {
        let mut unknown = test_process(
            710,
            1,
            "unknown-app",
            SystemTaskClass::Unknown,
            PriorityBand::Interactive,
            20,
        );
        unknown.voluntary_ctxt_switches_delta = 4;

        let snapshot = test_snapshot(vec![unknown]);
        let group = FocusGroup {
            kind: FocusGroupKind::Unknown,
            root_pids: vec![710],
            member_pids: vec![710],
            primary_pid: Some(710),
            display_name: "unknown-app".to_owned(),
            score: 0.50,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.50,
            priority_band: PriorityBand::Interactive,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::UnknownForegroundLike { pid: 710, comm } if comm == "unknown-app"
        )));
    }

    #[test]
    fn safety_warnings_report_broad_system_service_group() {
        let systemd = test_process(
            720,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        let dbus = test_process(
            721,
            720,
            "dbus-daemon",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        let network = test_process(
            722,
            720,
            "NetworkManager",
            SystemTaskClass::NetworkDaemon,
            PriorityBand::Background,
            0,
        );
        let storage = test_process(
            723,
            720,
            "udisksd",
            SystemTaskClass::StorageDaemon,
            PriorityBand::Background,
            0,
        );

        let snapshot = test_snapshot(vec![systemd, dbus, network, storage]);
        let group = FocusGroup {
            kind: FocusGroupKind::Idle,
            root_pids: vec![720],
            member_pids: vec![720, 721, 722, 723],
            primary_pid: Some(720),
            display_name: "systemd".to_owned(),
            score: 0.10,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.40,
            priority_band: PriorityBand::Background,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::TooBroadSystemServiceGroup { root_pids } if root_pids == &vec![720]
        )));
    }

    #[test]
    fn make_focus_group_appends_safety_warning_reasons() {
        let audio = test_process(
            730,
            1,
            "pipewire",
            SystemTaskClass::AudioRealtime,
            PriorityBand::CriticalRealtime,
            5,
        );

        let snapshot = test_snapshot(vec![audio]);
        let group = make_focus_group(
            &snapshot,
            FocusGroupKind::Desktop,
            vec![730],
            vec![730],
            Some(730),
            vec!["desktop group test".to_owned()],
        )
        .unwrap();

        assert!(
            group
                .reasons
                .iter()
                .any(|reason| reason.contains("safety: critical realtime/input process present"))
        );
    }
}
