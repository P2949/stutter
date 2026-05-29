use super::*;

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
            .and_then(|t| t.window_id.clone()),
        None
    );
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
    assert_eq!(snapshot.decision.target.as_ref().and_then(|t| t.pid), None);
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("firefox")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.class.clone())
            .as_deref(),
        Some("Firefox")
    );
    assert_eq!(snapshot.decision.confidence, 0.65);
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
            .and_then(|t| t.title.clone())
            .as_deref(),
        Some("unknown window")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.window_id.clone())
            .as_deref(),
        Some("9988")
    );
    assert_eq!(snapshot.decision.confidence, 0.35);
}

#[test]
fn sway_tree_parser_does_not_treat_empty_app_id_or_class_as_medium_confidence() {
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
                  "app_id": "   ",
                  "window": null,
                  "window_properties": {
                    "class": "",
                    "instance": "   ",
                    "title": ""
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
    let snapshot = provider.sample_from_tree_json(5_500, json);

    assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
    assert_eq!(snapshot.decision.target.as_ref().and_then(|t| t.pid), None);
    assert_eq!(snapshot.decision.confidence, 0.35);
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
    assert_eq!(snapshot.decision.confidence, 0.0);
    assert!(
        snapshot
            .decision
            .reasons
            .first()
            .map(|r| r.reason.clone())
            .unwrap_or_default()
            .contains("did not contain a focused node")
    );
}
