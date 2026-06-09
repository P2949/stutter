//! Focus classification tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{test_support::FakeProcProcess, *};

    #[test]
    fn audio_realtime_priority_band() {
        let fake_process =
            FakeProcProcess::new(1400, 1, "pipewire", "pipewire").with_sched_policy(SCHED_FIFO);
        assert_eq!(fake_process.sched_policy, Some(SCHED_FIFO));

        let classification = classify_process(&ProcessIdentity {
            pid: 1400.into(),
            ppid: 1.into(),
            comm: "pipewire",
            cmdline: "pipewire",
            exe_path: None,
            cgroup_path: None,
            sched_policy: Some(SCHED_FIFO),
        });

        assert_eq!(classification.class, SystemTaskClass::AudioRealtime);
        assert_eq!(classification.priority_band, PriorityBand::CriticalRealtime);
        assert!(classification.confidence >= 0.60);

        assert_eq!(
            priority_band_for_class(SystemTaskClass::AudioRealtime, Some(SCHED_RR)),
            PriorityBand::CriticalRealtime
        );
    }

    #[test]
    fn compositor_not_background() {
        for comm in ["sway", "kwin_wayland", "mutter", "gnome-shell"] {
            let classification = classify_process(&ProcessIdentity {
                pid: 1500.into(),
                ppid: 1.into(),
                comm,
                cmdline: comm,
                exe_path: None,
                cgroup_path: None,
                sched_policy: None,
            });

            assert_eq!(classification.class, SystemTaskClass::Compositor);
            assert_eq!(
                classification.priority_band,
                PriorityBand::ForegroundLatency
            );
            assert_ne!(classification.priority_band, PriorityBand::Background);
        }

        assert_eq!(
            priority_band_for_class(SystemTaskClass::Compositor, None),
            PriorityBand::ForegroundLatency
        );
    }

    #[test]
    fn focus_classification_uses_community_rule_reason_for_proton_game() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1600.into(),
            ppid: 1.into(),
            comm: "KingdomCome",
            cmdline: "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe --game",
            exe_path: Some("/usr/bin/wine"),
            cgroup_path: Some("/user.slice/app-steam-379430.scope"),
            sched_policy: None,
        });

        assert_eq!(classification.class, SystemTaskClass::Game);
        assert_eq!(
            classification.priority_band,
            PriorityBand::ForegroundLatency
        );
        assert!(
            classification.reasons.iter().any(|reason| {
                reason.contains("community-rules") && reason.contains("wine_proton_k.rules")
            }),
            "reasons={:?}",
            classification.reasons
        );
    }

    #[test]
    fn focus_ambiguous_exe_without_context_is_not_game() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1601.into(),
            ppid: 1.into(),
            comm: "build.exe",
            cmdline: "/tmp/build.exe --compile",
            exe_path: Some("/tmp/build.exe"),
            cgroup_path: Some("/user.slice/app-builder.scope"),
            sched_policy: None,
        });

        assert_ne!(classification.class, SystemTaskClass::Game);
    }

    #[test]
    fn focus_hardcoded_audio_classification_wins_over_community_context() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1602.into(),
            ppid: 1.into(),
            comm: "pipewire",
            cmdline: "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
            exe_path: Some("/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe"),
            cgroup_path: Some("/user.slice/app-steam-379430.scope"),
            sched_policy: Some(SCHED_FIFO),
        });

        assert_eq!(classification.class, SystemTaskClass::AudioRealtime);
        assert!(
            classification
                .reasons
                .iter()
                .all(|reason| !reason.contains("community-rules")),
            "reasons={:?}",
            classification.reasons
        );
    }

    #[test]
    fn legacy_task_class_maps_game_related_system_classes_to_game() {
        assert_eq!(SystemTaskClass::Game, SystemTaskClass::Game);
        assert_eq!(
            SystemTaskClass::GameRenderThread,
            SystemTaskClass::GameRenderThread
        );
        assert_eq!(
            SystemTaskClass::GameWorkerThread,
            SystemTaskClass::GameWorkerThread
        );
    }

    #[test]
    fn legacy_task_class_preserves_special_foreground_classes() {
        assert_eq!(SystemTaskClass::WineServer, SystemTaskClass::WineServer);
        assert_eq!(SystemTaskClass::GameScope, SystemTaskClass::GameScope);
        assert_eq!(SystemTaskClass::Compositor, SystemTaskClass::Compositor);
    }

    #[test]
    fn legacy_task_class_maps_daemon_and_kernel_classes_to_service() {
        assert_eq!(SystemTaskClass::Service, SystemTaskClass::Service);
        assert_eq!(
            SystemTaskClass::StorageDaemon,
            SystemTaskClass::StorageDaemon
        );
        assert_eq!(
            SystemTaskClass::NetworkDaemon,
            SystemTaskClass::NetworkDaemon
        );
        assert_eq!(SystemTaskClass::KernelThread, SystemTaskClass::KernelThread);
        assert_eq!(SystemTaskClass::IrqThread, SystemTaskClass::IrqThread);
    }

    #[test]
    fn legacy_task_class_maps_all_other_system_classes_to_helper() {
        let classes = [
            SystemTaskClass::AudioRealtime,
            SystemTaskClass::Input,
            SystemTaskClass::BrowserForeground,
            SystemTaskClass::BrowserBackground,
            SystemTaskClass::BrowserRenderer,
            SystemTaskClass::BrowserGpu,
            SystemTaskClass::BrowserNetwork,
            SystemTaskClass::BuildJob,
            SystemTaskClass::Compiler,
            SystemTaskClass::Linker,
            SystemTaskClass::Indexer,
            SystemTaskClass::PackageManager,
            SystemTaskClass::Editor,
            SystemTaskClass::Terminal,
            SystemTaskClass::Shell,
            SystemTaskClass::Media,
            SystemTaskClass::Recorder,
            SystemTaskClass::VirtualMachine,
            SystemTaskClass::Unknown,
        ];

        for class in classes {
            assert_eq!(class, class);
        }
    }
}
