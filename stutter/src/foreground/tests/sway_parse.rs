//! Sway foreground provider and parser tests extracted from `foreground`.
//!
//! Owns Sway tree parsing, Sway detection, Sway Wayland detection, and Sway title-redaction provider tests.
//! Does not own resolver policy, X11 parsing, Hyprland parsing, or production foreground behavior.

#[cfg(test)]
mod tests {
    use super::super::{super::*, SequenceProvider, restore_env_var};

    #[test]
    fn parse_sway_tree_finds_focused_node_with_pid() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "gaming",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Kingdom Come: Deliverance",
                  "type": "con",
                  "focused": true,
                  "pid": 4242,
                  "app_id": "steam_app_379430",
                  "window": 73400327,
                  "window_properties": {
                    "class": "steam_app_379430",
                    "instance": "steam_app_379430",
                    "title": "Kingdom Come: Deliverance"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(1_000, json);

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.class.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.window_id.as_deref(), Some("73400327"));
        assert_eq!(snapshot.workspace.as_deref(), Some("gaming"));
        assert!((snapshot.confidence - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_sway_tree_prefers_app_id_and_redacts_title_by_default() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "gaming",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Private browser tab title",
                  "type": "con",
                  "focused": true,
                  "pid": 9000,
                  "app_id": "firefox",
                  "window": 123,
                  "window_properties": {
                    "class": "Navigator",
                    "instance": "Navigator",
                    "title": "Private browser tab title"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(2_000, json);
        let event = snapshot.to_event(false).unwrap();

        assert_eq!(snapshot.app_id.as_deref(), Some("firefox"));
        assert_eq!(snapshot.class.as_deref(), Some("Navigator"));
        assert_eq!(snapshot.title.as_deref(), Some("Private browser tab title"));
        assert_eq!(event.app_id.as_deref(), Some("firefox"));
        assert_eq!(event.class.as_deref(), Some("Navigator"));
        assert_eq!(event.title, None);
    }

    #[test]
    fn sway_tree_parser_skips_focused_workspace_and_selects_focused_con() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "games",
              "type": "workspace",
              "focused": true,
              "nodes": [
                {
                  "id": 42,
                  "name": "Kingdom Come: Deliverance",
                  "type": "con",
                  "focused": true,
                  "pid": 4242,
                  "app_id": "steam_app_379430",
                  "window": null,
                  "window_properties": {
                    "class": "steam_app_379430",
                    "instance": "steam_app_379430",
                    "title": "Kingdom Come: Deliverance"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(1_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(snapshot.window_id.as_deref(), Some("42"));
        assert_eq!(snapshot.workspace.as_deref(), Some("games"));
    }

    #[test]
    fn sway_tree_parser_reports_unavailable_when_only_workspace_is_focused() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "games",
              "type": "workspace",
              "focused": true,
              "nodes": [],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(1_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id, None);
        assert_eq!(snapshot.window_id, None);
    }

    #[test]
    fn sway_tree_parser_prefers_deep_focused_leaf_over_focused_parent_container() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "dev",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 10,
                  "name": "split container",
                  "type": "con",
                  "focused": true,
                  "nodes": [
                    {
                      "id": 11,
                      "name": "Alacritty",
                      "type": "con",
                      "focused": true,
                      "pid": 9001,
                      "app_id": "Alacritty",
                      "window": null,
                      "window_properties": null,
                      "nodes": [],
                      "floating_nodes": []
                    }
                  ],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(1_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(9001));
        assert_eq!(snapshot.window_id.as_deref(), Some("11"));
        assert_eq!(snapshot.workspace.as_deref(), Some("dev"));
    }

    #[test]
    fn sway_provider_detection_uses_swaysock_environment() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous = std::env::var_os("SWAYSOCK");

        unsafe {
            std::env::remove_var("SWAYSOCK");
        }
        assert!(!SwayForegroundProvider::is_detected());

        unsafe {
            std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
        }
        assert!(SwayForegroundProvider::is_detected());

        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("SWAYSOCK", previous);
            } else {
                std::env::remove_var("SWAYSOCK");
            }
        }
    }

    #[test]
    fn sway_tree_parser_finds_focused_tiled_node_with_pid() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "games",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Steam - private title",
                  "type": "con",
                  "focused": true,
                  "pid": 4242,
                  "app_id": "steam",
                  "window": 12345,
                  "window_properties": {
                    "class": "Steam",
                    "instance": "steam",
                    "title": "Steam - private title"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(2_000, json);

        assert_eq!(snapshot.elapsed_ms, 2_000);
        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.app_id.as_deref(), Some("steam"));
        assert_eq!(snapshot.class.as_deref(), Some("Steam"));
        assert_eq!(snapshot.title.as_deref(), Some("Steam - private title"));
        assert_eq!(snapshot.window_id.as_deref(), Some("12345"));
        assert_eq!(snapshot.workspace.as_deref(), Some("games"));
        assert_eq!(snapshot.confidence, 0.95);
    }

    #[test]
    fn sway_tree_parser_finds_focused_floating_node() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "dev",
              "type": "workspace",
              "focused": false,
              "nodes": [],
              "floating_nodes": [
                {
                  "id": 9,
                  "name": "floating terminal",
                  "type": "con",
                  "focused": true,
                  "pid": 7777,
                  "app_id": "foot",
                  "window": null,
                  "window_properties": {
                    "class": "foot",
                    "instance": "foot",
                    "title": "floating terminal"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ]
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(3_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, Some(7777));
        assert_eq!(snapshot.app_id.as_deref(), Some("foot"));
        assert_eq!(snapshot.window_id.as_deref(), Some("9"));
        assert_eq!(snapshot.workspace.as_deref(), Some("dev"));
        assert_eq!(snapshot.confidence, 0.95);
    }

    #[test]
    fn sway_tree_parser_uses_medium_confidence_without_pid() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "web",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "Firefox",
                  "type": "con",
                  "focused": true,
                  "pid": null,
                  "app_id": "firefox",
                  "window": null,
                  "window_properties": {
                    "class": "Firefox",
                    "instance": "Navigator",
                    "title": "Private tab title"
                  },
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(4_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id.as_deref(), Some("firefox"));
        assert_eq!(snapshot.class.as_deref(), Some("Firefox"));
        assert_eq!(snapshot.confidence, 0.65);
    }

    #[test]
    fn sway_tree_parser_uses_low_confidence_for_title_or_window_only() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [
            {
              "id": 2,
              "name": "misc",
              "type": "workspace",
              "focused": false,
              "nodes": [
                {
                  "id": 3,
                  "name": "unknown window",
                  "type": "con",
                  "focused": true,
                  "pid": null,
                  "app_id": null,
                  "window": 9988,
                  "window_properties": null,
                  "nodes": [],
                  "floating_nodes": []
                }
              ],
              "floating_nodes": []
            }
          ],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(5_000, json);

        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.pid, None);
        assert_eq!(snapshot.app_id, None);
        assert_eq!(snapshot.class, None);
        assert_eq!(snapshot.title.as_deref(), Some("unknown window"));
        assert_eq!(snapshot.window_id.as_deref(), Some("9988"));
        assert_eq!(snapshot.confidence, 0.35);
    }

    #[test]
    fn sway_tree_parser_reports_unavailable_when_no_focused_node_exists() {
        let json = r#"
        {
          "id": 1,
          "name": "root",
          "type": "root",
          "focused": false,
          "nodes": [],
          "floating_nodes": []
        }
        "#;

        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(6_000, json);

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Unavailable);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(snapshot.reason.contains("did not contain a focused node"));
    }

    #[test]
    fn sway_tree_parser_reports_error_for_invalid_json() {
        let provider = SwayForegroundProvider::new();
        let snapshot = provider.sample_from_tree_json(7_000, "{not-json}");

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Error);
        assert_eq!(snapshot.confidence, 0.0);
        assert!(
            snapshot
                .reason
                .contains("failed to parse swaymsg get_tree JSON")
        );
    }

    #[test]
    fn sway_provider_titles_are_redacted_by_resolver_default() {
        let provider = SequenceProvider::new(
            ForegroundSource::Sway,
            vec![ForegroundWindowSnapshot {
                elapsed_ms: 0,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Available,
                pid: Some(4242),
                app_id: Some("steam".to_owned()),
                class: Some("Steam".to_owned()),
                title: Some("Sensitive foreground title".to_owned()),
                window_id: Some("12345".to_owned()),
                workspace: Some("games".to_owned()),
                confidence: 0.95,
                stale_ms: None,
                reason: "test sway provider snapshot".to_owned(),
            }],
        );
        let mut resolver = ForegroundResolver::new(Box::new(provider));

        let snapshot = resolver.sample(8_000);

        assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
        assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
        assert_eq!(snapshot.title, None);
    }

    #[test]
    fn sway_wayland_is_not_treated_as_generic_unsupported_wayland() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
        let previous_swaysock = std::env::var_os("SWAYSOCK");
        let previous_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");

        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "wayland-1");
            std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
        }

        assert!(!is_generic_wayland_without_supported_foreground_api());

        unsafe {
            restore_env_var("WAYLAND_DISPLAY", previous_wayland_display);
            restore_env_var("SWAYSOCK", previous_swaysock);
            restore_env_var("HYPRLAND_INSTANCE_SIGNATURE", previous_hyprland);
        }
    }
}
