use super::topology::CandidateCpuLayout;
use crate::{
    process_tree::{CompiledPattern, TaskClass},
    profiles::{Profile, ProfileRule},
};

pub(crate) fn baseline_online_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "baseline-online".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(layout.online_mask.clone()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: Vec::new(),
        }],
    }
}

pub(crate) fn game_isolate_render_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-isolate-render".to_owned(),
        rules: vec![
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.wine_server_mask,
                vec![TaskClass::WineServer],
                Vec::new(),
            ),
            profile_rule(
                &layout.worker_mask,
                vec![TaskClass::GameWorkerThread, TaskClass::GameHelper],
                Vec::new(),
            ),
            profile_rule(&layout.worker_mask, vec![TaskClass::Game], Vec::new()),
        ],
    }
}

pub(crate) fn game_compositor_separate_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-compositor-separate".to_owned(),
        rules: vec![
            profile_rule(
                &layout.separate_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.separate_game_mask,
                vec![
                    TaskClass::Game,
                    TaskClass::GameRenderThread,
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

pub(crate) fn helper_spread_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "helper-spread".to_owned(),
        rules: vec![profile_rule(
            &layout.helper_mask,
            vec![
                TaskClass::GameHelper,
                TaskClass::GameWorkerThread,
                TaskClass::SteamRuntime,
                TaskClass::Helper,
            ],
            Vec::new(),
        )],
    }
}

pub(crate) fn wine_server_dedicated_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "wine-server-dedicated".to_owned(),
        rules: vec![profile_rule(
            &layout.wine_server_mask,
            vec![TaskClass::WineServer],
            Vec::new(),
        )],
    }
}

pub(crate) fn avoid_smt_contention_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "avoid-smt-contention".to_owned(),
        rules: vec![
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.avoid_smt_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_worker_mask,
                vec![
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

pub(crate) fn profile_rule(
    affinity: &crate::affinity::CpuMask,
    match_class: Vec<TaskClass>,
    match_comm: Vec<&str>,
) -> ProfileRule {
    ProfileRule {
        affinity: Some(affinity.clone()),
        nice: None,
        ionice: None,
        match_class,
        match_comm: match_comm
            .into_iter()
            .map(|pattern| {
                CompiledPattern::new(pattern.to_owned())
                    // invariant: generated profile patterns are static literals validated by tests.
                    .expect("generated candidate command pattern must be valid")
            })
            .collect(),
    }
}
