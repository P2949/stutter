use super::*;

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
    assert_eq!(
        snapshot.decision.target.as_ref().and_then(|t| t.pid),
        Some(4242)
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("steam_app_379430")
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
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("73400327")
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
    assert!((snapshot.decision.confidence - 0.95).abs() < f32::EPSILON);
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
    assert_eq!(
        snapshot.decision.target.as_ref().and_then(|t| t.pid),
        Some(4242)
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("42")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.workspace.clone())
            .as_deref(),
        Some("games")
    );
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
    assert_eq!(
        snapshot.decision.target.as_ref().and_then(|t| t.pid),
        Some(9001)
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("11")
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
    assert_eq!(
        snapshot.decision.target.as_ref().and_then(|t| t.pid),
        Some(4242)
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("steam")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.class.clone())
            .as_deref(),
        Some("Steam")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.title.clone())
            .as_deref(),
        Some("Steam - private title")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("12345")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.workspace.clone())
            .as_deref(),
        Some("games")
    );
    assert_eq!(snapshot.decision.confidence, 0.95);
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
    assert_eq!(
        snapshot.decision.target.as_ref().and_then(|t| t.pid),
        Some(7777)
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("foot")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("9")
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
    assert_eq!(snapshot.decision.confidence, 0.95);
}
