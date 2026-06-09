use super::topology::CoreChoice;
use crate::{process_tree::TaskClass, profiles::ProfileRule, topology::sorted_unique};

pub(crate) fn same_core(left: &CoreChoice, right: &CoreChoice) -> bool {
    left.package_id == right.package_id
        && left.core_id == right.core_id
        && left.numa_node == right.numa_node
}

pub(crate) fn flatten_core_cpus(cores: &[CoreChoice]) -> Vec<u32> {
    sorted_unique(
        cores
            .iter()
            .flat_map(|core| core.cpus.iter().copied())
            .collect(),
    )
}

pub(crate) fn rule_matches_render_or_main_game(rule: &ProfileRule) -> bool {
    rule.match_class.contains(&TaskClass::GameRenderThread)
        || (rule.match_class.contains(&TaskClass::Game)
            && rule.match_comm.iter().any(|pattern| {
                let raw = pattern.raw().to_ascii_lowercase();
                raw.contains("render") || raw.contains("main")
            }))
}

pub(crate) fn rule_matches_game_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::Game
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::WineServer
        )
    })
}

pub(crate) fn rule_matches_compositor_or_gamescope(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::Compositor | TaskClass::GameScope))
}

pub(crate) fn rule_matches_background_or_helper_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::SteamRuntime
                | TaskClass::Helper
                | TaskClass::WineServer
        )
    })
}

pub(crate) fn rule_matches_audio_or_input(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::AudioRealtime | TaskClass::Input))
}

pub(crate) fn is_baseline_online_profile(name: &str) -> bool {
    name == "baseline-online"
}
