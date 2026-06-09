use super::*;

#[test]
fn mangohud_timing_defaults_preserve_existing_behavior() {
    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: None,
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(merged.mangohud.tail_idle_sleep_ms, 75);
    assert_eq!(merged.mangohud.alignment_poll_ms, 500);
    assert_eq!(merged.alerts.desktop_timeout_ms, 10_000);
}
