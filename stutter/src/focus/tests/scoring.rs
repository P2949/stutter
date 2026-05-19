//! Focus scoring tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{
        test_support::{group_test_process as test_process, group_test_snapshot as test_snapshot},
        *,
    };

    #[test]
    fn focus_group_score_is_clamped_and_exposes_breakdown() {
        let cargo = test_process(
            600,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            10_000,
        );
        let rustc = test_process(
            601,
            600,
            "rustc",
            SystemTaskClass::Compiler,
            PriorityBand::Throughput,
            10_000,
        );

        let snapshot = test_snapshot(vec![cargo, rustc]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.score, 1.0);
        assert!(compile.score_breakdown.cpu_score > 0.0);
        assert!(compile.score_breakdown.class_priority_score > 0.0);
        assert!(compile.score_breakdown.stability_score > 0.0);
    }

    #[test]
    fn focus_group_confidence_is_not_high_from_name_only() {
        let cargo = test_process(
            610,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            0,
        );

        let snapshot = test_snapshot(vec![cargo]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert!(compile.score_breakdown.class_priority_score > 0.0);
        assert_eq!(compile.score_breakdown.cpu_score, 0.0);
        assert!(compile.confidence <= 0.55);
    }

    #[test]
    fn focus_group_penalizes_indexer_only_compile_group() {
        let clangd = test_process(
            620,
            1,
            "clangd",
            SystemTaskClass::Indexer,
            PriorityBand::Background,
            25,
        );

        let snapshot = test_snapshot(vec![clangd]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert!(compile.score_breakdown.penalty >= 0.55);
        assert!(compile.score < 0.50);
    }

    #[test]
    fn focus_group_scores_game_from_runtime_and_active_descendants() {
        let mut runtime = test_process(
            630,
            1,
            "pressure-vessel",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            5,
        );
        runtime.cmdline = "/home/user/.steam/steamapps/common/Game/pressure-vessel".to_owned();

        let mut game = test_process(
            631,
            630,
            "Game.exe",
            SystemTaskClass::GameRenderThread,
            PriorityBand::ForegroundLatency,
            80,
        );
        game.cmdline = "/home/user/.steam/steamapps/common/Game/Game.exe".to_owned();

        let snapshot = test_snapshot(vec![runtime, game]);
        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert!(game_group.score > 0.50);
        assert!(game_group.confidence > 0.55);
        assert!(game_group.score_breakdown.class_priority_score > 0.0);
        assert_eq!(game_group.score_breakdown.penalty, 0.0);
    }

    #[test]
    fn focus_group_penalizes_launcher_only_game_group() {
        let steam = test_process(
            640,
            1,
            "steam",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            0,
        );

        let snapshot = test_snapshot(vec![steam]);
        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert!(game_group.score_breakdown.penalty >= 0.20);
        assert!(game_group.confidence <= 0.55);
    }

    #[test]
    fn focus_group_scores_browser_from_active_children() {
        let parent = test_process(
            650,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            5,
        );
        let renderer = test_process(
            651,
            650,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            60,
        );
        let gpu = test_process(
            652,
            650,
            "GPU Process",
            SystemTaskClass::BrowserGpu,
            PriorityBand::Interactive,
            20,
        );

        let snapshot = test_snapshot(vec![parent, renderer, gpu]);
        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert!(browser.score > 0.40);
        assert!(browser.confidence > 0.55);
        assert_eq!(browser.score_breakdown.penalty, 0.0);
    }

    #[test]
    fn focus_group_penalizes_many_idle_browser_renderers() {
        let parent = test_process(
            660,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            1,
        );
        let renderer_one = test_process(
            661,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_two = test_process(
            662,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_three = test_process(
            663,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_four = test_process(
            664,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );

        let snapshot = test_snapshot(vec![
            parent,
            renderer_one,
            renderer_two,
            renderer_three,
            renderer_four,
        ]);
        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert!(browser.score_breakdown.penalty > 0.0);
        assert!(browser.confidence <= 0.75);
    }

    #[test]
    fn focus_group_kind_maps_system_classes() {
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::GameRenderThread),
            FocusGroupKind::Game
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::BrowserGpu),
            FocusGroupKind::Browser
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compiler),
            FocusGroupKind::Compile
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Media),
            FocusGroupKind::Media
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Recorder),
            FocusGroupKind::Recording
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::VirtualMachine),
            FocusGroupKind::VirtualMachine
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compositor),
            FocusGroupKind::Desktop
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Service),
            FocusGroupKind::Idle
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Unknown),
            FocusGroupKind::Unknown
        );
    }
}
