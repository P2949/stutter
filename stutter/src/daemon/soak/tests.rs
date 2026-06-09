#[cfg(test)]
mod tests {
    use crate::daemon::soak::{
        DaemonSoakBudget, DaemonSoakConfig, DaemonSoakProfile, SoakScenario, run_fake_daemon_soak,
        run_scenario_daemon_soak,
    };

    #[test]
    fn observe_only_fake_soak_passes_default_budgets() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            duration_seconds: 60,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert_eq!(report.profile, DaemonSoakProfile::ObserveOnly);
        assert_eq!(report.metrics.event_drops, 0);
        assert_eq!(report.metrics.scenario_count, 1);
    }

    #[test]
    fn fake_low_risk_soak_tracks_actions_and_rollbacks() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            profile: DaemonSoakProfile::ApplyLowRiskFake,
            duration_seconds: 120,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert!(report.metrics.fake_actions_started >= 1);
        assert!(report.metrics.fake_rollbacks >= 1);
    }

    #[test]
    fn fake_soak_fails_when_budget_is_too_small() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            duration_seconds: 60,
            budget: DaemonSoakBudget {
                max_disk_growth_bytes: 1,
                ..DaemonSoakBudget::default()
            },
            ..DaemonSoakConfig::default()
        });

        assert!(!report.passed);
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.reason_code == "disk_growth")
        );
    }

    #[test]
    fn soak_profile_parses_aliases() {
        assert_eq!(
            "low-risk-fake".parse::<DaemonSoakProfile>().unwrap(),
            DaemonSoakProfile::ApplyLowRiskFake
        );
    }

    #[test]
    fn scenario_driven_soak_fixtures_preserve_safety_invariants() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("testdata/autotune/soak");
        let mut paths = std::fs::read_dir(&fixture_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        assert_eq!(paths.len(), 12);

        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let scenario: SoakScenario = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            let report = run_scenario_daemon_soak(
                &DaemonSoakConfig {
                    profile: DaemonSoakProfile::ApplyLowRiskFake,
                    duration_seconds: scenario.ticks.len() as u64,
                    ..DaemonSoakConfig::default()
                },
                std::slice::from_ref(&scenario),
            );

            assert!(
                report.passed,
                "{} failed: {:?}",
                scenario.name, report.failures
            );
            assert!(
                report.metrics.event_drops <= DaemonSoakBudget::default().max_event_drops,
                "{} event drops exceeded budget",
                scenario.name
            );
            assert!(
                report.metrics.max_event_queue_len
                    <= DaemonSoakBudget::default().max_event_queue_len,
                "{} queue accounting should stay bounded",
                scenario.name
            );
            assert!(
                report.metrics.max_active_experiments <= 1,
                "{} active experiment invariant failed",
                scenario.name
            );
            assert_eq!(report.scenarios.len(), 1);
            assert!(report.scenarios[0].passed);
        }
    }
}
