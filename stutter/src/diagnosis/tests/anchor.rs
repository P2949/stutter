use super::*;

#[test]
fn cluster_anchor_follows_ranked_game_primary() {
    // compositor 3ms + game 10ms => diagnosis primary GameThreadSchedulerDelay and anchor_kind Game
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);

    let anchor = select_anchor_for_diagnosis(&cluster, &d);
    assert_eq!(anchor.kind, ClusterAnchorKind::Game);
    assert_eq!(anchor.task, 456);
}

#[test]
fn cluster_anchor_follows_compositor_primary() {
    // compositor 6ms + game 5ms => diagnosis primary CompositorSchedulerDelay and anchor_kind Compositor
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);

    let anchor = select_anchor_for_diagnosis(&cluster, &d);
    assert_eq!(anchor.kind, ClusterAnchorKind::Compositor);
    assert_eq!(anchor.task, 123);
}
