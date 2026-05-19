//! X11 foreground provider and parser tests extracted from `foreground`.
//!
//! Owns X11 xprop parsing, X11 provider availability checks, and X11 title-redaction provider tests.
//! Does not own Sway parsing, Hyprland parsing, resolver policy, or production foreground behavior.

#[cfg(test)]
mod tests {
    use super::super::{super::*, SequenceProvider};

    #[test]
    fn parse_xprop_active_window_pid_and_class() {
        let root_output = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4600007\n";
        let properties_output = r#"
_NET_WM_PID(CARDINAL) = 12345
WM_CLASS(STRING) = "steam_app_379430", "steam_app_379430"
_NET_WM_NAME(UTF8_STRING) = "Kingdom Come: Deliverance"
WM_NAME(STRING) = "Kingdom Come: Deliverance"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(4_000, root_output, properties_output);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(12345));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.class.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.window_id.as_deref(), Some("0x4600007"));
        assert_eq!(snapshot.title.as_deref(), Some("Kingdom Come: Deliverance"));
        assert!((snapshot.confidence - 0.90).abs() < f32::EPSILON);
    }

    #[test]
    fn x11_provider_reports_unavailable_without_display() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous = std::env::var_os("DISPLAY");

        unsafe {
            std::env::remove_var("DISPLAY");
        }

        let mut provider = X11ForegroundProvider::new();
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(snapshot.reason.contains("DISPLAY is not set"));

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("DISPLAY", previous);
            } else {
                std::env::remove_var("DISPLAY");
            }
        }
    }

    #[test]
    fn x11_provider_checks_xprop_before_sampling() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous = std::env::var_os("DISPLAY");

        unsafe {
            std::env::set_var("DISPLAY", ":99");
        }

        let mut provider =
            X11ForegroundProvider::new().with_xprop("stutter-definitely-missing-xprop-binary");
        let snapshot = provider.sample(1_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(
            snapshot
                .reason
                .contains("stutter-definitely-missing-xprop-binary")
        );
        assert!(snapshot.reason.contains("was not found in PATH"));

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("DISPLAY", previous);
            } else {
                std::env::remove_var("DISPLAY");
            }
        }
    }

    #[test]
    fn x11_parser_extracts_pid_class_and_title_with_high_confidence() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4600007\n";
        let props = r#"
_NET_WM_PID(CARDINAL) = 12345
WM_CLASS(STRING) = "steam_app_379430", "steam_app_379430"
_NET_WM_NAME(UTF8_STRING) = "Kingdom Come: Deliverance"
WM_NAME(STRING) = "fallback title"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(2_000, active, props);

        assert_eq!(snapshot.elapsed_ms, 2_000);
        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(12345));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.class.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.title.as_deref(), Some("Kingdom Come: Deliverance"));
        assert_eq!(snapshot.window_id.as_deref(), Some("0x4600007"));
        assert_eq!(snapshot.workspace, None);
        assert_eq!(snapshot.confidence, 0.90);
        assert_eq!(snapshot.reason, "active X11 window from xprop");
    }

    #[test]
    fn x11_parser_uses_wm_name_fallback_when_net_wm_name_missing() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x1200007\n";
        let props = r#"
WM_CLASS(STRING) = "foot", "foot"
WM_NAME(STRING) = "terminal title"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(3_000, active, props);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("foot"));
        assert_eq!(snapshot.class.as_deref(), Some("foot"));
        assert_eq!(snapshot.title.as_deref(), Some("terminal title"));
        assert_eq!(snapshot.confidence, 0.55);
    }

    #[test]
    fn x11_parser_uses_medium_confidence_for_wm_class_without_pid() {
        let active = "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x4600007\n";
        let props = r#"
WM_CLASS(STRING) = "Navigator", "Firefox"
_NET_WM_NAME(UTF8_STRING) = "Private browser tab"
"#;

        let provider = X11ForegroundProvider::new();
        let snapshot = provider.sample_from_xprop_outputs(4_000, active, props);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("Navigator"));
        assert_eq!(snapshot.class.as_deref(), Some("Firefox"));
        assert_eq!(snapshot.title.as_deref(), Some("Private browser tab"));
        assert_eq!(snapshot.confidence, 0.55);
    }

    #[test]
    fn x11_parser_reports_unavailable_for_missing_active_window() {
        let provider = X11ForegroundProvider::new();

        for active in [
            "_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n",
            "_NET_ACTIVE_WINDOW:  not found.\n",
            "unrelated xprop output\n",
        ] {
            let snapshot = provider.sample_from_xprop_outputs(
                5_000,
                active,
                "WM_CLASS(STRING) = \"steam\", \"Steam\"\n",
            );

            assert_eq!(snapshot.source, Some(ForegroundSource::X11));
            assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
            assert_eq!(snapshot.confidence, 0.0);
            assert!(
                snapshot
                    .reason
                    .contains("did not contain an active X11 window")
            );
        }
    }

    #[test]
    fn x11_parser_handles_escaped_quoted_strings() {
        let values = parse_x11_quoted_strings(r#"WM_CLASS(STRING) = "term\"inal", "Class\\Name""#);

        assert_eq!(
            values,
            vec!["term\"inal".to_owned(), "Class\\Name".to_owned()]
        );
    }

    #[test]
    fn x11_provider_titles_are_redacted_by_resolver_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::X11,
            vec![ForegroundWindowSnapshot {
                elapsed_ms: 0,
                source: Some(ForegroundSource::X11),
                status: ForegroundProviderStatus::Available,
                pid: Some(12345),
                app_id: Some("steam_app_379430".to_owned()),
                class: Some("steam_app_379430".to_owned()),
                title: Some("Kingdom Come: Deliverance".to_owned()),
                window_id: Some("0x4600007".to_owned()),
                workspace: None,
                confidence: 0.90,
                stale_ms: None,
                reason: "test x11 provider snapshot".to_owned(),
            }],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(6_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::X11));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.title, None);
    }
}
