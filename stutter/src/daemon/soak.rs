use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonSoakProfile {
    ObserveOnly,
    ApplyLowRiskFake,
}

impl DaemonSoakProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe-only",
            Self::ApplyLowRiskFake => "apply-low-risk-fake",
        }
    }
}

impl fmt::Display for DaemonSoakProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonSoakProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "observe-only" | "observe" => Ok(Self::ObserveOnly),
            "apply-low-risk-fake" | "low-risk-fake" => Ok(Self::ApplyLowRiskFake),
            other => anyhow::bail!(
                "unknown soak profile {other:?}; expected observe-only or apply-low-risk-fake"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakConfig {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub tick_millis: u64,
    pub budget: DaemonSoakBudget,
}

impl Default for DaemonSoakConfig {
    fn default() -> Self {
        Self {
            profile: DaemonSoakProfile::ObserveOnly,
            duration_seconds: 60,
            tick_millis: 1_000,
            budget: DaemonSoakBudget::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakBudget {
    pub max_memory_growth_bytes: u64,
    pub max_file_descriptors: u64,
    pub max_disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub max_task_count: u64,
    pub max_history_bytes: u64,
    pub max_cpu_millis_per_second: u64,
    pub max_wakeups_per_second: u64,
    pub max_event_drops: u64,
}

impl Default for DaemonSoakBudget {
    fn default() -> Self {
        Self {
            max_memory_growth_bytes: 8 * 1024 * 1024,
            max_file_descriptors: 128,
            max_disk_growth_bytes: 32 * 1024 * 1024,
            max_event_queue_len: 4096,
            max_task_count: 2048,
            max_history_bytes: 16 * 1024 * 1024,
            max_cpu_millis_per_second: 25,
            max_wakeups_per_second: 30,
            max_event_drops: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakReport {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub ticks: u64,
    pub passed: bool,
    pub metrics: DaemonSoakMetrics,
    pub failures: Vec<DaemonSoakFailure>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakMetrics {
    pub memory_growth_bytes: u64,
    pub file_descriptors: u64,
    pub disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub task_count: u64,
    pub history_bytes: u64,
    pub cpu_millis_per_second: u64,
    pub wakeups_per_second: u64,
    pub event_drops: u64,
    pub fake_actions_started: u64,
    pub fake_rollbacks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakFailure {
    pub reason_code: String,
    pub message: String,
}

pub fn run_fake_daemon_soak(config: &DaemonSoakConfig) -> DaemonSoakReport {
    let tick_millis = config.tick_millis.max(1);
    let ticks = config
        .duration_seconds
        .saturating_mul(1_000)
        .saturating_add(tick_millis - 1)
        / tick_millis;
    let ticks = ticks.max(1);
    let mut metrics = DaemonSoakMetrics {
        file_descriptors: 12,
        task_count: 1,
        cpu_millis_per_second: 2,
        wakeups_per_second: 1_000 / tick_millis,
        ..DaemonSoakMetrics::default()
    };

    for tick in 0..ticks {
        let incoming_events: u64 = match config.profile {
            DaemonSoakProfile::ObserveOnly => 6,
            DaemonSoakProfile::ApplyLowRiskFake => 10,
        };
        let processed_events: u64 = match config.profile {
            DaemonSoakProfile::ObserveOnly => 6,
            DaemonSoakProfile::ApplyLowRiskFake => 9,
        };
        let queue_len = incoming_events.saturating_sub(processed_events);

        metrics.max_event_queue_len = metrics.max_event_queue_len.max(queue_len);
        metrics.memory_growth_bytes = metrics
            .memory_growth_bytes
            .saturating_add(memory_growth_per_tick(config.profile, tick));
        metrics.disk_growth_bytes = metrics
            .disk_growth_bytes
            .saturating_add(disk_growth_per_tick(config.profile));
        metrics.history_bytes = metrics
            .history_bytes
            .saturating_add(history_growth_per_tick(config.profile));
        metrics.task_count = metrics.task_count.max(1 + (tick % 32));

        if config.profile == DaemonSoakProfile::ApplyLowRiskFake && tick % 60 == 10 {
            metrics.fake_actions_started = metrics.fake_actions_started.saturating_add(1);
        }
        if config.profile == DaemonSoakProfile::ApplyLowRiskFake && tick % 60 == 40 {
            metrics.fake_rollbacks = metrics.fake_rollbacks.saturating_add(1);
        }
    }

    metrics.file_descriptors += match config.profile {
        DaemonSoakProfile::ObserveOnly => 4,
        DaemonSoakProfile::ApplyLowRiskFake => 8,
    };
    metrics.cpu_millis_per_second += match config.profile {
        DaemonSoakProfile::ObserveOnly => 3,
        DaemonSoakProfile::ApplyLowRiskFake => 7,
    };
    metrics.wakeups_per_second += match config.profile {
        DaemonSoakProfile::ObserveOnly => 2,
        DaemonSoakProfile::ApplyLowRiskFake => 4,
    };

    let failures = evaluate_soak_failures(&metrics, &config.budget);

    DaemonSoakReport {
        profile: config.profile,
        duration_seconds: config.duration_seconds,
        ticks,
        passed: failures.is_empty(),
        metrics,
        failures,
    }
}

fn memory_growth_per_tick(profile: DaemonSoakProfile, tick: u64) -> u64 {
    match profile {
        DaemonSoakProfile::ObserveOnly => {
            if tick.is_multiple_of(120) {
                1024
            } else {
                0
            }
        }
        DaemonSoakProfile::ApplyLowRiskFake => {
            if tick.is_multiple_of(60) {
                2048
            } else {
                0
            }
        }
    }
}

fn disk_growth_per_tick(profile: DaemonSoakProfile) -> u64 {
    match profile {
        DaemonSoakProfile::ObserveOnly => 128,
        DaemonSoakProfile::ApplyLowRiskFake => 256,
    }
}

fn history_growth_per_tick(profile: DaemonSoakProfile) -> u64 {
    match profile {
        DaemonSoakProfile::ObserveOnly => 96,
        DaemonSoakProfile::ApplyLowRiskFake => 192,
    }
}

fn evaluate_soak_failures(
    metrics: &DaemonSoakMetrics,
    budget: &DaemonSoakBudget,
) -> Vec<DaemonSoakFailure> {
    let mut failures = Vec::new();

    check_budget(
        &mut failures,
        "memory_growth",
        metrics.memory_growth_bytes,
        budget.max_memory_growth_bytes,
    );
    check_budget(
        &mut failures,
        "file_descriptors",
        metrics.file_descriptors,
        budget.max_file_descriptors,
    );
    check_budget(
        &mut failures,
        "disk_growth",
        metrics.disk_growth_bytes,
        budget.max_disk_growth_bytes,
    );
    check_budget(
        &mut failures,
        "event_queue",
        metrics.max_event_queue_len,
        budget.max_event_queue_len,
    );
    check_budget(
        &mut failures,
        "task_count",
        metrics.task_count,
        budget.max_task_count,
    );
    check_budget(
        &mut failures,
        "history_size",
        metrics.history_bytes,
        budget.max_history_bytes,
    );
    check_budget(
        &mut failures,
        "cpu_overhead",
        metrics.cpu_millis_per_second,
        budget.max_cpu_millis_per_second,
    );
    check_budget(
        &mut failures,
        "wakeups",
        metrics.wakeups_per_second,
        budget.max_wakeups_per_second,
    );
    check_budget(
        &mut failures,
        "event_drops",
        metrics.event_drops,
        budget.max_event_drops,
    );

    failures
}

fn check_budget(
    failures: &mut Vec<DaemonSoakFailure>,
    reason_code: &'static str,
    observed: u64,
    limit: u64,
) {
    if observed <= limit {
        return;
    }

    failures.push(DaemonSoakFailure {
        reason_code: reason_code.to_owned(),
        message: format!("observed {observed} exceeds budget {limit}"),
    });
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    fn observe_only_fake_soak_passes_default_budgets() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            duration_seconds: 60,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert_eq!(report.profile, DaemonSoakProfile::ObserveOnly);
        assert_eq!(report.metrics.event_drops, 0);
    }

    #[test]
    fn fake_low_risk_soak_tracks_actions_and_rollbacks() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            profile: DaemonSoakProfile::ApplyLowRiskFake,
            duration_seconds: 120,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert!(report.metrics.fake_actions_started >= 2);
        assert!(report.metrics.fake_rollbacks >= 2);
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

    #[derive(Debug, Deserialize)]
    struct SoakFixture {
        name: String,
        profile: DaemonSoakProfile,
        duration_seconds: u64,
        expect_actions: bool,
        expect_rollbacks: bool,
        forbid_high_risk_apply: bool,
    }

    #[test]
    fn autonomous_watcher_soak_fixtures_preserve_safety_invariants() {
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

        assert_eq!(paths.len(), 10);

        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let fixture: SoakFixture = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            let report = run_fake_daemon_soak(&DaemonSoakConfig {
                profile: fixture.profile,
                duration_seconds: fixture.duration_seconds,
                ..DaemonSoakConfig::default()
            });

            assert!(
                report.passed,
                "{} failed: {:?}",
                fixture.name, report.failures
            );
            assert_eq!(report.metrics.event_drops, 0, "{}", fixture.name);
            assert!(
                report.metrics.max_event_queue_len
                    <= DaemonSoakBudget::default().max_event_queue_len,
                "{} queue accounting should stay bounded",
                fixture.name
            );
            assert_eq!(
                report.metrics.fake_actions_started > 0,
                fixture.expect_actions,
                "{} action expectation mismatch",
                fixture.name
            );
            assert_eq!(
                report.metrics.fake_rollbacks > 0,
                fixture.expect_rollbacks,
                "{} rollback expectation mismatch",
                fixture.name
            );
            if fixture.expect_actions {
                assert!(
                    report.metrics.fake_rollbacks <= report.metrics.fake_actions_started,
                    "{} rollbacks cannot exceed started actions",
                    fixture.name
                );
            }
            if fixture.forbid_high_risk_apply {
                assert_ne!(
                    report.profile.as_str(),
                    "apply-high-risk",
                    "{}",
                    fixture.name
                );
            }
        }
    }
}
