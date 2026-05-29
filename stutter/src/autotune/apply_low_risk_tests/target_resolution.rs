//! Target-resolution tests extracted from `autotune::apply_low_risk`.
//!
//! Owns candidate selection from profiles and target tree PID resolution tests.
//! Does not own policy gates, experiment resolution, audit/journal behavior, rollback orchestration, or production behavior.

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use super::super::*;
    use crate::{
        actions::SafetyClass,
        autotune::planning::{candidate::CandidateAction, dry_run::CandidateDryRunRecord},
        profiles::Profile,
    };

    #[test]
    fn low_risk_planner_selects_first_eligible_record_and_documents_empty_profiles() {
        let skipped = CandidateAction::cpu_affinity_profile(
            Profile {
                name: "background".to_owned(),
                rules: Vec::new(),
            },
            4_242,
        );
        let selected = CandidateAction::cpu_affinity_profile(
            Profile {
                name: "game-main".to_owned(),
                rules: Vec::new(),
            },
            4_242,
        );
        let records = vec![
            CandidateDryRunRecord {
                candidate_name: "background".to_owned(),
                affected_tasks: 0,
                warnings: Vec::new(),
                safety_class: SafetyClass::ReversibleLowRisk,
                eligible: false,
                reason: Some("no matching tasks".to_owned()),
            },
            CandidateDryRunRecord {
                candidate_name: "game-main".to_owned(),
                affected_tasks: 31,
                warnings: Vec::new(),
                safety_class: SafetyClass::ReversibleLowRisk,
                eligible: true,
                reason: None,
            },
        ];

        let chosen = select_first_eligible_low_risk_candidate(
            &[skipped.clone(), selected.clone()],
            &records,
        )
        .unwrap();
        assert_eq!(chosen.profile_name(), "game-main");

        let plan = ApplyLowRiskPlan {
            tree_pid: 4_242,
            profiles_path: PathBuf::from("/tmp/profiles.toml"),
            candidate: chosen,
            dry_run_record: records[1].clone(),
            duration: Duration::from_secs(3),
        };
        assert_eq!(plan.tree_pid, 4_242);
        assert_eq!(plan.profiles_path, PathBuf::from("/tmp/profiles.toml"));
        assert_eq!(plan.candidate.profile_name(), "game-main");
        assert_eq!(plan.dry_run_record.affected_tasks, 31);
        assert_eq!(plan.duration, Duration::from_secs(3));

        let err = plan_apply_low_risk_from_profiles(
            4_242,
            Path::new("/tmp/profiles.toml"),
            &[],
            Duration::ZERO,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no CPU affinity profile candidates were generated"));
    }

    #[tokio::test]
    async fn apply_low_risk_command_requires_a_target_selector_before_loading_profiles() {
        let input = crate::autotune::commands::live::AutotuneCommandInput {
            config: None,
            watch_process: None,
            tree_pid: None,
            profiles: None,
            mode: crate::daemon::policy::DaemonMode::ApplyLowRisk,
            decision_log: None,
            duration_seconds: Some(0),
            washout_seconds: 0,
            washout_verify_interval_ms: 1,
            summary_ms: 1_000,
            preset: "game".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            focus_source: crate::config::FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: crate::config::ForegroundSource::Auto,
            foreground_poll_ms: 1_000,
            foreground_max_stale_ms: 5_000,
            allow_system_wide_suggestions: false,
            allow_medium_risk: false,
            high_risk_dry_run: false,
            dry_run_all_safe: false,
        };

        let err = apply_low_risk_command(&input)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly one target selector"));
    }

    #[test]
    fn target_selector_requires_exactly_one_selector() {
        let err = resolve_one_target_tree_pid_at(Path::new("/proc"), Some(1), Some("Game.exe"))
            .unwrap_err()
            .to_string();

        assert!(err.contains("exactly one target selector"));
    }

    #[test]
    fn watch_process_requires_exactly_one_match() {
        let dir = temp_dir("watch-process-many");
        fake_proc_with_comm(&dir, 10, "Game.exe");
        fake_proc_with_comm(&dir, 11, "Game.exe");

        let err = resolve_one_target_tree_pid_at(&dir, None, Some("Game.exe"))
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires one active target tree"));
        fs::remove_dir_all(dir).ok();
    }
}
