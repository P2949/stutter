//! Hyprland and unsupported Wayland foreground tests extracted from `foreground`.
//!
//! Owns Hyprland active-window parsing and generic Wayland/Hyprland unsupported-provider selection tests.
//! Does not own Sway parsing, X11 parsing, resolver policy, or production foreground behavior.

#[cfg(test)]
mod tests {
    use super::super::{super::*, restore_env_var};

    #[test]
    fn parse_hyprland_activewindow_extracts_pid_class_workspace() {
        let json = r#"
        {
          "address": "0x123456789abcdef",
          "mapped": true,
          "hidden": false,
          "at": [10, 20],
          "size": [1920, 1080],
          "workspace": {
            "id": 3,
            "name": "gaming"
          },
          "floating": false,
          "monitor": 0,
          "class": "steam_app_379430",
          "initialClass": "steam_app_379430",
          "title": "Kingdom Come: Deliverance",
          "initialTitle": "Kingdom Come: Deliverance",
          "pid": 4242,
          "xwayland": false
        }
        "#;

        let snapshot = hyprland_snapshot_from_activewindow_json(3_000, json);
        let event = snapshot.to_event(false).unwrap();

        assert_eq!(snapshot.source, Some(ForegroundSource::Hyprland));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.class.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.workspace.as_deref(), Some("gaming"));
        assert_eq!(snapshot.window_id.as_deref(), Some("0x123456789abcdef"));
        assert!((snapshot.confidence - 0.95).abs() < f32::EPSILON);
        assert_eq!(event.title, None);
    }

    #[test]
    fn unsupported_provider_reports_generic_wayland_reason() {
        let mut provider = UnsupportedForegroundProvider::generic_wayland();

        let snapshot = provider.sample(1_234);

        assert_eq!(snapshot.elapsed_ms, 1_234);
        assert_eq!(snapshot.source, Some(ForegroundSource::Unsupported));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id, None);
        assert_eq!(snapshot.class, None);
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.window_id, None);
        assert_eq!(snapshot.workspace, None);
        assert_eq!(snapshot.confidence, 0.0);
        assert_eq!(snapshot.reason, GENERIC_WAYLAND_UNSUPPORTED_REASON);
    }

    #[test]
    fn generic_wayland_without_sway_or_hyprland_is_unsupported() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(2_000);

        assert_eq!(provider.source(), ForegroundSource::Unsupported);
        assert_eq!(snapshot.source, Some(ForegroundSource::Unsupported));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(
            snapshot.reason,
            "GNOME/KDE Wayland session detected, but no safe generic Wayland foreground-window API is available"
        );
        assert_eq!(snapshot.title, None);

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
        }
    }

    #[test]
    fn kde_wayland_without_compositor_specific_provider_is_unsupported() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(3_000);

        assert_eq!(provider.source(), ForegroundSource::Unsupported);
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(
            snapshot.reason,
            "GNOME/KDE Wayland session detected, but no safe generic Wayland foreground-window API is available"
        );

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
        }
    }

    #[test]
    fn hyprland_wayland_is_reserved_for_future_compositor_specific_provider() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::remove_var("SWAYSOCK");
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        assert!(!is_generic_wayland_without_supported_foreground_api());

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }
}
