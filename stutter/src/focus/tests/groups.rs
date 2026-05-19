//! Focus grouping tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{
        test_support::{
            FakeProcProcess, first_focus_group, focus_snapshot_from_fake_proc,
            group_test_process as test_process, group_test_snapshot as test_snapshot,
            situation_mapping_test_group,
        },
        *,
    };

    #[test]
    fn maps_focus_groups_to_autotune_situations() {
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Game)),
            SituationKind::GameFocused
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Browser)),
            SituationKind::BrowserFocused
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Compile)),
            SituationKind::CompileLoad
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Media)),
            SituationKind::MediaPlayback
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Recording)),
            SituationKind::Recording
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(
                FocusGroupKind::VirtualMachine
            )),
            SituationKind::VirtualMachineLoad
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Idle)),
            SituationKind::Idle
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Desktop)),
            SituationKind::Unknown
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Unknown)),
            SituationKind::Unknown
        );
    }

    #[test]
    fn focus_groups_prefer_stable_compile_root_over_compiler_children() {
        let snapshot = test_snapshot(vec![
            test_process(
                10,
                1,
                "foot",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                1,
            ),
            test_process(
                11,
                10,
                "cargo",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                2,
            ),
            test_process(
                12,
                11,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                80,
            ),
            test_process(
                13,
                11,
                "ld.lld",
                SystemTaskClass::Linker,
                PriorityBand::Throughput,
                25,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![11]);
        assert_eq!(compile.primary_pid, Some(11));
        assert_eq!(compile.member_pids, vec![11, 12, 13]);
    }

    #[test]
    fn focus_groups_group_orphan_compilers_under_nearest_terminal_session() {
        let snapshot = test_snapshot(vec![
            test_process(
                20,
                1,
                "kitty",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                3,
            ),
            test_process(
                21,
                20,
                "zsh",
                SystemTaskClass::Shell,
                PriorityBand::Interactive,
                4,
            ),
            test_process(
                22,
                21,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                60,
            ),
            test_process(
                23,
                21,
                "clang",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                50,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![21]);
        assert_eq!(compile.primary_pid, Some(22));
        assert_eq!(compile.member_pids, vec![21, 22, 23]);
    }

    #[test]
    fn focus_groups_root_browser_at_parent_not_idle_renderer() {
        let snapshot = test_snapshot(vec![
            test_process(
                30,
                1,
                "firefox",
                SystemTaskClass::BrowserForeground,
                PriorityBand::ForegroundLatency,
                5,
            ),
            test_process(
                31,
                30,
                "Web Content",
                SystemTaskClass::BrowserRenderer,
                PriorityBand::Interactive,
                100,
            ),
            test_process(
                32,
                30,
                "GPU Process",
                SystemTaskClass::BrowserGpu,
                PriorityBand::Interactive,
                20,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert_eq!(browser.root_pids, vec![30]);
        assert_eq!(browser.primary_pid, Some(30));
        assert_eq!(browser.member_pids, vec![30, 31, 32]);
    }

    #[test]
    fn focus_groups_include_wineserver_tied_to_game_runtime() {
        let mut game = test_process(
            40,
            1,
            "pressure-vessel",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            10,
        );
        game.cmdline = "/home/user/.steam/steamapps/common/Game/pressure-vessel".to_owned();
        game.cgroup_path = Some(std::path::PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut game_child = test_process(
            41,
            40,
            "Game.exe",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            120,
        );
        game_child.cmdline = "/home/user/.steam/steamapps/common/Game/Game.exe".to_owned();
        game_child.cgroup_path = Some(std::path::PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut wineserver = test_process(
            42,
            1,
            "wineserver",
            SystemTaskClass::WineServer,
            PriorityBand::ForegroundLatency,
            15,
        );
        wineserver.cgroup_path = Some(std::path::PathBuf::from("/user.slice/app-steam-game.scope"));

        let snapshot = test_snapshot(vec![game, game_child, wineserver]);

        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert_eq!(game_group.root_pids, vec![40]);
        assert_eq!(game_group.primary_pid, Some(40));
        assert_eq!(game_group.member_pids, vec![40, 41, 42]);
    }

    #[test]
    fn focus_groups_do_not_let_idle_steam_beat_active_compile() {
        let mut steam = test_process(
            50,
            1,
            "steam",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        steam.cmdline = "steam".to_owned();

        let cargo = test_process(
            60,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            30,
        );
        let rustc = test_process(
            61,
            60,
            "rustc",
            SystemTaskClass::Compiler,
            PriorityBand::Throughput,
            90,
        );

        let snapshot = test_snapshot(vec![steam, cargo, rustc]);

        let groups = build_focus_groups(&snapshot);

        assert_eq!(groups.first().unwrap().kind, FocusGroupKind::Compile);
        assert!(
            groups
                .iter()
                .position(|group| group.kind == FocusGroupKind::Idle)
                .unwrap()
                > groups
                    .iter()
                    .position(|group| group.kind == FocusGroupKind::Compile)
                    .unwrap()
        );
    }

    #[test]
    fn focus_groups_fallback_selects_highest_non_service_interactive_tree_by_cpu() {
        let service = test_process(
            70,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            500,
        );
        let editor = test_process(
            80,
            1,
            "nvim",
            SystemTaskClass::Editor,
            PriorityBand::Interactive,
            20,
        );
        let terminal = test_process(
            90,
            1,
            "foot",
            SystemTaskClass::Terminal,
            PriorityBand::Interactive,
            60,
        );

        let mut snapshot = test_snapshot(vec![service, editor, terminal]);
        for process in snapshot.processes.values_mut() {
            process.classification.class = SystemTaskClass::Unknown;
        }
        snapshot
            .processes
            .get_mut(&70)
            .unwrap()
            .classification
            .class = SystemTaskClass::Service;
        snapshot
            .processes
            .get_mut(&80)
            .unwrap()
            .classification
            .class = SystemTaskClass::Editor;
        snapshot
            .processes
            .get_mut(&90)
            .unwrap()
            .classification
            .class = SystemTaskClass::Terminal;

        let groups = build_focus_groups(&snapshot);
        let fallback = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Unknown)
            .unwrap();

        assert_eq!(fallback.root_pids, vec![90]);
        assert_eq!(fallback.primary_pid, Some(90));
        assert_eq!(fallback.member_pids, vec![90]);
    }

    #[test]
    fn gaming_wins_over_idle_steam() {
        let cgroup = "/user.slice/app-steam-game.scope";
        let first_sample = vec![
            FakeProcProcess::new(100, 1, "steam", "steam").with_cgroup("/user.slice/steam.scope"),
            FakeProcProcess::new(
                110,
                99, // Use a different PPID so they aren't siblings
                "pressure-vessel",
                "/home/user/.steam/steamapps/common/Game/pressure-vessel",
            )
            .with_cgroup(cgroup),
            FakeProcProcess::new(
                111,
                110,
                "Game.exe",
                "/home/user/.steam/steamapps/common/Game/Game.exe",
            )
            .with_cgroup(cgroup),
        ];
        let second_sample = vec![
            FakeProcProcess::new(100, 1, "steam", "steam").with_cgroup("/user.slice/steam.scope"),
            FakeProcProcess::new(
                110,
                99,
                "pressure-vessel",
                "/home/user/.steam/steamapps/common/Game/pressure-vessel",
            )
            .with_cgroup(cgroup)
            .with_activity(5, 0, 0, 1, 0),
            FakeProcProcess::new(
                111,
                110,
                "Game.exe",
                "/home/user/.steam/steamapps/common/Game/Game.exe",
            )
            .with_cgroup(cgroup)
            .with_activity(180, 0, 0, 20, 5),
        ];

        let snapshot = focus_snapshot_from_fake_proc(
            "gaming_wins_over_idle_steam",
            first_sample,
            second_sample,
        );

        assert_eq!(snapshot.groups.first().unwrap().kind, FocusGroupKind::Game);
        let game_group = first_focus_group(&snapshot, FocusGroupKind::Game);
        assert_eq!(game_group.root_pids, vec![110]);
        assert_eq!(game_group.primary_pid, Some(110));
        assert!(game_group.member_pids.contains(&110));
        assert!(game_group.member_pids.contains(&111));
        assert!(!game_group.member_pids.contains(&100));
    }

    #[test]
    fn idle_launcher_does_not_win() {
        let first_sample = vec![
            FakeProcProcess::new(200, 1, "steam", "steam"),
            FakeProcProcess::new(210, 1, "firefox", "firefox"),
            FakeProcProcess::new(211, 210, "firefox: Web Content", "firefox web content"),
            FakeProcProcess::new(
                212,
                210,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            ),
        ];
        let second_sample = vec![
            FakeProcProcess::new(200, 1, "steam", "steam"),
            FakeProcProcess::new(210, 1, "firefox", "firefox").with_activity(15, 0, 0, 8, 1),
            FakeProcProcess::new(211, 210, "firefox: Web Content", "firefox web content")
                .with_activity(160, 0, 0, 30, 2),
            FakeProcProcess::new(
                212,
                210,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            )
            .with_activity(30, 0, 0, 12, 1),
        ];

        let snapshot = focus_snapshot_from_fake_proc(
            "idle_launcher_does_not_win",
            first_sample,
            second_sample,
        );

        assert_eq!(
            snapshot.groups.first().unwrap().kind,
            FocusGroupKind::Browser
        );
        let browser_group = first_focus_group(&snapshot, FocusGroupKind::Browser);
        assert_eq!(browser_group.root_pids, vec![210]);
        assert_eq!(browser_group.primary_pid, Some(210));
        assert!(browser_group.member_pids.contains(&211));
        assert!(browser_group.member_pids.contains(&212));
    }

    #[test]
    fn compile_root_is_stable() {
        let first_sample = vec![
            FakeProcProcess::new(300, 1, "cargo", "cargo build"),
            FakeProcProcess::new(301, 300, "rustc", "rustc crate_a"),
            FakeProcProcess::new(302, 300, "rustc", "rustc crate_b"),
        ];
        let second_sample = vec![
            FakeProcProcess::new(300, 1, "cargo", "cargo build").with_activity(20, 0, 0, 8, 1),
            FakeProcProcess::new(301, 300, "rustc", "rustc crate_a")
                .with_activity(200, 0, 0, 10, 2),
            FakeProcProcess::new(302, 300, "rustc", "rustc crate_b").with_activity(180, 0, 0, 9, 2),
        ];

        let snapshot =
            focus_snapshot_from_fake_proc("compile_root_is_stable", first_sample, second_sample);

        let compile_group = first_focus_group(&snapshot, FocusGroupKind::Compile);
        assert_eq!(compile_group.root_pids, vec![300]);
        assert_eq!(compile_group.primary_pid, Some(300));
        assert_eq!(compile_group.member_pids, vec![300, 301, 302]);
        assert!(!compile_group.root_pids.contains(&301));
        assert!(!compile_group.root_pids.contains(&302));
    }

    #[test]
    fn linker_pressure_sets_compile_linker_situation() {
        let first_sample = vec![
            FakeProcProcess::new(400, 1, "cargo", "cargo build"),
            FakeProcProcess::new(401, 400, "rustc", "rustc crate_a"),
            FakeProcProcess::new(402, 400, "ld.lld", "ld.lld -o target/debug/app"),
        ];
        let second_sample =
            vec![
                FakeProcProcess::new(400, 1, "cargo", "cargo build").with_activity(20, 0, 0, 8, 1),
                FakeProcProcess::new(401, 400, "rustc", "rustc crate_a")
                    .with_activity(120, 0, 0, 10, 2),
                FakeProcProcess::new(402, 400, "ld.lld", "ld.lld -o target/debug/app")
                    .with_activity(250, 64 * 1024 * 1024, 128 * 1024 * 1024, 8, 2),
            ];

        let snapshot = focus_snapshot_from_fake_proc(
            "linker_pressure_sets_compile_linker_situation",
            first_sample,
            second_sample,
        );

        let compile_group = first_focus_group(&snapshot, FocusGroupKind::Compile);
        assert_eq!(compile_group.root_pids, vec![400]);
        assert!(compile_group.member_pids.contains(&402));
        assert!(compile_group.score_breakdown.io_score > 0.0);
        assert!(
            compile_group
                .reasons
                .iter()
                .any(|reason| reason.contains("compile group prefers stable build roots"))
        );
        assert!(matches!(
            situation_for_group(compile_group),
            SituationKind::CompileLoad | SituationKind::CompileLinkerPressure
        ));
    }

    #[test]
    fn browser_renderer_grouping() {
        let first_sample = vec![
            FakeProcProcess::new(500, 1, "firefox", "firefox"),
            FakeProcProcess::new(
                501,
                500,
                "firefox: Web Content",
                "firefox isolated web content",
            ),
            FakeProcProcess::new(502, 500, "firefox: Web Content", "firefox web content tab"),
            FakeProcProcess::new(
                503,
                500,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            ),
        ];
        let second_sample = vec![
            FakeProcProcess::new(500, 1, "firefox", "firefox").with_activity(10, 0, 0, 5, 1),
            FakeProcProcess::new(
                501,
                500,
                "firefox: Web Content",
                "firefox isolated web content",
            )
            .with_activity(90, 0, 0, 20, 2),
            FakeProcProcess::new(502, 500, "firefox: Web Content", "firefox web content tab")
                .with_activity(70, 0, 0, 16, 2),
            FakeProcProcess::new(
                503,
                500,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            )
            .with_activity(30, 0, 0, 8, 1),
        ];

        let snapshot =
            focus_snapshot_from_fake_proc("browser_renderer_grouping", first_sample, second_sample);

        let browser_group = first_focus_group(&snapshot, FocusGroupKind::Browser);
        assert_eq!(browser_group.root_pids, vec![500]);
        assert_eq!(browser_group.primary_pid, Some(500));
        assert_eq!(browser_group.member_pids, vec![500, 501, 502, 503]);
    }
}
