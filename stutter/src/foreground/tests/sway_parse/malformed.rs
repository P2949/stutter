use super::*;

#[test]
fn sway_tree_parser_reports_error_for_invalid_json() {
    let provider = SwayForegroundProvider::new();
    let snapshot = provider.sample_from_tree_json(7_000, "{not-json}");

    assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
    assert_eq!(snapshot.status, ForegroundProviderStatus::Error);
    assert_eq!(snapshot.decision.confidence, 0.0);
    assert!(
        snapshot
            .decision
            .primary_reason()
            .unwrap_or_default()
            .contains("failed to parse swaymsg get_tree JSON")
    );
}

#[test]
fn sway_wayland_is_not_treated_as_generic_unsupported_wayland() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
    let previous_swaysock = std::env::var_os("SWAYSOCK");
    let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

    // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
        std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
        std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    }

    assert!(!is_generic_wayland_without_supported_foreground_api());

    // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
    unsafe {
        restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
        restore_env_var("SWAYSOCK", previous_swaysock);
        restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
    }
}
