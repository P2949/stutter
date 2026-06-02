//! Hyprland and unsupported Wayland foreground tests extracted from `foreground`.
//!
//! Owns Hyprland active-window parsing and generic Wayland/Hyprland unsupported-provider selection tests.
//! Does not own Sway parsing, X11 parsing, resolver policy, or production foreground behavior.

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

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
        assert_eq!(
            snapshot.decision.target.as_ref().and_then(|t| t.pid),
            Some(4242)
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.class.clone())
                .as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.workspace.clone())
                .as_deref(),
            Some("gaming")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.window_id.clone())
                .as_deref(),
            Some("0x123456789abcdef")
        );
        assert!((snapshot.decision.confidence - 0.95).abs() < f32::EPSILON);
        assert_eq!(
            event.decision.target.as_ref().and_then(|t| t.title.clone()),
            None
        );
    }

    #[test]
    fn hyprland_is_detected_when_instance_signature_is_set() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        assert!(HyprlandForegroundProvider::is_detected());

        // SAFETY: TEST_MUTEX is still held and previous_hyprland was captured before mutation.
        unsafe {
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }

    #[test]
    fn hyprland_provider_sample_uses_hyprctl_activewindow_json() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let root = crate::test_support::TestRoot::new("hyprland-provider-sample");
        let hyprctl = root.join("hyprctl");
        fs::write(
            &hyprctl,
            r#"#!/bin/sh
cat <<'JSON'
{
  "address": "0xabcdef",
  "workspace": { "name": "dev" },
  "class": "kitty",
  "title": "Terminal",
  "pid": 8128
}
JSON
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&hyprctl).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hyprctl, permissions).unwrap();

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        let mut provider =
            HyprlandForegroundProvider::new().with_hyprctl(hyprctl.to_string_lossy());
        let snapshot = provider.sample(4_500);

        assert_eq!(provider.source(), ForegroundSource::Hyprland);
        assert_eq!(snapshot.elapsed_ms, 4_500);
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(
            snapshot.decision.target.as_ref().and_then(|t| t.pid),
            Some(8128)
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.class.clone())
                .as_deref(),
            Some("kitty")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.workspace.clone())
                .as_deref(),
            Some("dev")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.window_id.clone())
                .as_deref(),
            Some("0xabcdef")
        );

        // SAFETY: TEST_MUTEX is still held and previous_hyprland was captured before mutation.
        unsafe {
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }

    #[test]
    fn hyprland_provider_missing_helper_degrades_cleanly() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        let mut provider =
            HyprlandForegroundProvider::new().with_hyprctl("stutter-missing-hyprctl-helper");
        let snapshot = provider.sample(5_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::Hyprland));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default()
                .contains("stutter-missing-hyprctl-helper")
        );
        assert!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default()
                .contains("trusted foreground helper paths")
        );

        // SAFETY: TEST_MUTEX is still held and previous_hyprland was captured before mutation.
        unsafe {
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }

    #[test]
    fn unsupported_provider_reports_generic_wayland_reason() {
        let mut provider = UnsupportedForegroundProvider::generic_wayland();

        let snapshot = provider.sample(1_234);

        assert_eq!(snapshot.elapsed_ms, 1_234);
        assert_eq!(snapshot.source, Some(ForegroundSource::Unsupported));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unsupported);
        assert_eq!(snapshot.decision.target.as_ref().and_then(|t| t.pid), None);
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.app_id.clone()),
            None
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.class.clone()),
            None
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.window_id.clone()),
            None
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.workspace.clone()),
            None
        );
        assert_eq!(snapshot.decision.confidence, 0.0);
        assert_eq!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default(),
            GENERIC_WAYLAND_UNSUPPORTED_REASON
        );
    }

    #[test]
    fn gnome_wayland_without_helper_uses_gnome_provider_with_safe_unavailable_reason() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let previous_session_desktop = std::env::var_os("XDG_SESSION_DESKTOP");
        let previous_desktop_session = std::env::var_os("DESKTOP_SESSION");
        let previous_gdm_session = std::env::var_os("GDMSESSION");
        let previous_kde_session = std::env::var_os("KDE_FULL_SESSION");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
            std::env::remove_var("XDG_SESSION_DESKTOP");
            std::env::remove_var("DESKTOP_SESSION");
            std::env::remove_var("GDMSESSION");
            std::env::remove_var("KDE_FULL_SESSION");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(2_000);

        assert_eq!(provider.source(), ForegroundSource::Gnome);
        assert_eq!(snapshot.source, Some(ForegroundSource::Gnome));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default()
                .contains("unsafe org.gnome.Shell Eval is intentionally not used")
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|t| t.title.clone()),
            None
        );

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
            restore_env_var("XDG_SESSION_DESKTOP", previous_session_desktop);
            restore_env_var("DESKTOP_SESSION", previous_desktop_session);
            restore_env_var("GDMSESSION", previous_gdm_session);
            restore_env_var("KDE_FULL_SESSION", previous_kde_session);
        }
    }

    #[test]
    fn kde_wayland_without_helper_uses_kde_provider_with_safe_unavailable_reason() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let previous_display = std::env::var_os("DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let previous_session_desktop = std::env::var_os("XDG_SESSION_DESKTOP");
        let previous_desktop_session = std::env::var_os("DESKTOP_SESSION");
        let previous_gdm_session = std::env::var_os("GDMSESSION");
        let previous_kde_session = std::env::var_os("KDE_FULL_SESSION");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::remove_var("SWAYSOCK");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
            std::env::set_var("DISPLAY", ":0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
            std::env::remove_var("XDG_SESSION_DESKTOP");
            std::env::remove_var("DESKTOP_SESSION");
            std::env::remove_var("GDMSESSION");
            std::env::set_var("KDE_FULL_SESSION", "true");
        }

        assert!(is_generic_wayland_without_supported_foreground_api());
        assert!(current_desktop_looks_like_gnome_or_kde());

        let mut provider = auto_foreground_provider();
        let snapshot = provider.sample(3_000);

        assert_eq!(provider.source(), ForegroundSource::Kde);
        assert_eq!(snapshot.source, Some(ForegroundSource::Kde));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert!(
            snapshot
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default()
                .contains("KWin script injection is intentionally not used")
        );

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
            restore_env_var("DISPLAY", previous_display);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
            restore_env_var("XDG_SESSION_DESKTOP", previous_session_desktop);
            restore_env_var("DESKTOP_SESSION", previous_desktop_session);
            restore_env_var("GDMSESSION", previous_gdm_session);
            restore_env_var("KDE_FULL_SESSION", previous_kde_session);
        }
    }

    #[test]
    fn hyprland_wayland_uses_hyprland_foreground_provider() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::remove_var("SWAYSOCK");
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "fake-hyprland-instance");
        }

        assert!(!is_generic_wayland_without_supported_foreground_api());
        assert!(HyprlandForegroundProvider::is_detected());

        let provider = auto_foreground_provider();
        assert_eq!(provider.source(), ForegroundSource::Hyprland);

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }
}
