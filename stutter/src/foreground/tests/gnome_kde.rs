//! GNOME/KDE foreground provider safety and helper-contract tests.

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use super::super::{super::*, restore_env_var};

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn gnome_helper_json_extracts_foreground_identity_and_redacts_title_in_event() {
        let json = r#"{
            "pid": 4242,
            "app_id": "org.gnome.Terminal",
            "class": "Gnome-terminal",
            "title": "private shell title",
            "window_id": "gnome-window-1",
            "workspace": "1",
            "confidence": 0.91,
            "reason": "active GNOME window from extension helper"
        }"#;

        let provider = GnomeForegroundProvider::new();
        let snapshot = provider.sample_from_helper_json(10_000, json);
        let event = snapshot.to_event(false).unwrap();

        assert_eq!(snapshot.source, Some(ForegroundSource::Gnome));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|target| target.pid),
            Some(4242)
        );
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|target| target.app_id.clone())
                .as_deref(),
            Some("org.gnome.Terminal")
        );
        assert_eq!(
            event
                .decision
                .target
                .as_ref()
                .and_then(|target| target.title.clone()),
            None
        );
    }

    #[test]
    fn kde_helper_json_extracts_foreground_identity() {
        let json = r#"{
            "pid": 5151,
            "app_id": "org.kde.konsole",
            "class": "konsole",
            "title": "private title",
            "window_id": "kwin-window-7",
            "workspace": "dev"
        }"#;

        let provider = KdeForegroundProvider::new();
        let snapshot = provider.sample_from_helper_json(20_000, json);

        assert_eq!(snapshot.source, Some(ForegroundSource::Kde));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|target| target.pid),
            Some(5151)
        );
        assert!(
            snapshot.decision.confidence >= 0.65,
            "provider should assign useful confidence from pid/app/class"
        );
    }

    #[test]
    fn malformed_gnome_helper_json_reports_error() {
        let provider = GnomeForegroundProvider::new();
        let snapshot = provider.sample_from_helper_json(1_000, "{not json");

        assert_eq!(snapshot.source, Some(ForegroundSource::Gnome));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Error);
        assert!(
            snapshot
                .decision
                .primary_reason()
                .unwrap_or_default()
                .contains("failed to parse GNOME foreground helper JSON")
        );
    }

    #[test]
    fn gnome_provider_missing_helper_degrades_without_eval() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let previous_session_desktop = std::env::var_os("XDG_SESSION_DESKTOP");
        let previous_desktop_session = std::env::var_os("DESKTOP_SESSION");
        let previous_gdm_session = std::env::var_os("GDMSESSION");
        let previous_kde_session = std::env::var_os("KDE_FULL_SESSION");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
            std::env::remove_var("XDG_SESSION_DESKTOP");
            std::env::remove_var("DESKTOP_SESSION");
            std::env::remove_var("GDMSESSION");
            std::env::remove_var("KDE_FULL_SESSION");
        }

        let mut provider =
            GnomeForegroundProvider::new().with_helper("stutter-missing-gnome-helper");
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::Gnome));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        let reason = snapshot.decision.primary_reason().unwrap_or_default();
        assert!(reason.contains("stutter-missing-gnome-helper"));
        assert!(reason.contains("unsafe org.gnome.Shell Eval is intentionally not used"));

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
            restore_env_var("XDG_SESSION_DESKTOP", previous_session_desktop);
            restore_env_var("DESKTOP_SESSION", previous_desktop_session);
            restore_env_var("GDMSESSION", previous_gdm_session);
            restore_env_var("KDE_FULL_SESSION", previous_kde_session);
        }
    }

    #[test]
    fn kde_provider_missing_helper_degrades_without_kwin_script_injection() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let previous_session_desktop = std::env::var_os("XDG_SESSION_DESKTOP");
        let previous_desktop_session = std::env::var_os("DESKTOP_SESSION");
        let previous_gdm_session = std::env::var_os("GDMSESSION");
        let previous_kde_session = std::env::var_os("KDE_FULL_SESSION");

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
            std::env::remove_var("XDG_SESSION_DESKTOP");
            std::env::remove_var("DESKTOP_SESSION");
            std::env::remove_var("GDMSESSION");
            std::env::set_var("KDE_FULL_SESSION", "true");
        }

        let mut provider = KdeForegroundProvider::new().with_helper("stutter-missing-kde-helper");
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::Kde));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        let reason = snapshot.decision.primary_reason().unwrap_or_default();
        assert!(reason.contains("stutter-missing-kde-helper"));
        assert!(reason.contains("KWin script injection is intentionally not used"));

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
            restore_env_var("XDG_SESSION_DESKTOP", previous_session_desktop);
            restore_env_var("DESKTOP_SESSION", previous_desktop_session);
            restore_env_var("GDMSESSION", previous_gdm_session);
            restore_env_var("KDE_FULL_SESSION", previous_kde_session);
        }
    }

    #[test]
    fn gnome_provider_runs_trusted_helper_when_available() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland = std::env::var_os("WAYLAND_DISPLAY");
        let previous_desktop = std::env::var_os("XDG_CURRENT_DESKTOP");
        let previous_session_desktop = std::env::var_os("XDG_SESSION_DESKTOP");
        let previous_desktop_session = std::env::var_os("DESKTOP_SESSION");
        let previous_gdm_session = std::env::var_os("GDMSESSION");
        let previous_kde_session = std::env::var_os("KDE_FULL_SESSION");

        let root = crate::test_support::TestRoot::new("gnome-foreground-helper");
        let helper = root.join("stutter-gnome-foreground");
        write_executable(
            &helper,
            r#"#!/bin/sh
cat <<'JSON'
{"pid":7001,"app_id":"org.gnome.Nautilus","class":"Nautilus","window_id":"gnome-1"}
JSON
"#,
        );

        // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
            std::env::remove_var("XDG_SESSION_DESKTOP");
            std::env::remove_var("DESKTOP_SESSION");
            std::env::remove_var("GDMSESSION");
            std::env::remove_var("KDE_FULL_SESSION");
        }

        let mut provider = GnomeForegroundProvider::new().with_helper(helper.to_string_lossy());
        let snapshot = provider.sample(2_000);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(
            snapshot
                .decision
                .target
                .as_ref()
                .and_then(|target| target.pid),
            Some(7001)
        );

        // SAFETY: TEST_MUTEX is still held and previous values were captured before mutation.
        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland);
            restore_env_var("XDG_CURRENT_DESKTOP", previous_desktop);
            restore_env_var("XDG_SESSION_DESKTOP", previous_session_desktop);
            restore_env_var("DESKTOP_SESSION", previous_desktop_session);
            restore_env_var("GDMSESSION", previous_gdm_session);
            restore_env_var("KDE_FULL_SESSION", previous_kde_session);
        }
    }
}
