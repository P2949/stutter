use super::*;

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
        Some("Navigator")
    );
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.title.clone())
            .as_deref(),
        Some("Private browser tab title")
    );
    assert_eq!(
        event
            .decision
            .target
            .as_ref()
            .and_then(|t| t.app_id.clone())
            .as_deref(),
        Some("firefox")
    );
    assert_eq!(
        event
            .decision
            .target
            .as_ref()
            .and_then(|t| t.class.clone())
            .as_deref(),
        Some("Navigator")
    );
    assert_eq!(
        event.decision.target.as_ref().and_then(|t| t.title.clone()),
        None
    );
}

#[test]
fn sway_provider_titles_are_redacted_by_resolver_default() {
    let provider = SequenceProvider::new(
        ForegroundSource::Sway,
        vec![ForegroundWindowSnapshot::available(
            ForegroundAvailableInput {
                elapsed_ms: 0,
                source: ForegroundSource::Sway,
                pid: Some(4242),
                app_id: Some("steam".to_owned()),
                class: Some("Steam".to_owned()),
                title: Some("Sensitive foreground title".to_owned()),
                include_title: true,
                window_id: Some("12345".to_owned()),
                workspace: Some("games".to_owned()),
                confidence: 0.95,
                reason: "test sway provider snapshot".to_owned(),
            },
        )],
    );
    let mut resolver = ForegroundResolver::new(Box::new(provider));

    let snapshot = resolver.sample(8_000);

    assert_eq!(snapshot.source, Some(ForegroundSource::Sway));
    assert_eq!(snapshot.status, ForegroundProviderStatus::Available);
    assert_eq!(
        snapshot
            .decision
            .target
            .as_ref()
            .and_then(|t| t.title.clone()),
        None
    );
}
