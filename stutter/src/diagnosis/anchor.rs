//! Diagnosis anchor selection; this module owns spike-point role classification and anchor choice.

use super::{
    ClusterAnchor, ClusterAnchorKind, Diagnosis, StutterCause, model::SCHED_DELAY_SIGNIFICANT_NS,
};
use crate::{
    process_tree::TaskClass,
    spike::{SpikeCluster, SpikePoint},
};

pub(super) fn is_compositor_point(p: &SpikePoint) -> bool {
    matches!(p.class, TaskClass::Compositor | TaskClass::GameScope)
}

pub(super) fn is_game_point(p: &SpikePoint) -> bool {
    matches!(
        p.class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer
    ) || matches!(p.comm.as_str(), "Main" | "RenderThread")
}

pub(crate) fn select_anchor_for_diagnosis(
    cluster: &SpikeCluster,
    diagnosis: &Diagnosis,
) -> ClusterAnchor {
    if diagnosis.cause == StutterCause::GameThreadSchedulerDelay {
        let game_anchor = cluster
            .points
            .iter()
            .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = game_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Game,
            };
        }
    }

    if diagnosis.cause == StutterCause::CompositorSchedulerDelay {
        let compositor_anchor = cluster
            .points
            .iter()
            .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = compositor_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Compositor,
            };
        }
    }

    select_anchor(cluster)
}

pub(crate) fn select_anchor(cluster: &SpikeCluster) -> ClusterAnchor {
    // 1. Prefer highest-latency TaskClass::Compositor or TaskClass::GameScope point above 2ms.
    let compositor_anchor = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Compositor,
        };
    }

    // 2. Else prefer highest-latency Game/RenderThread/Main point above 2ms.
    let game_anchor = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Game,
        };
    }

    // 3. Else use highest-latency point.
    let fallback = cluster
        .points
        .iter()
        .max_by_key(|p| p.latency_ns)
        // invariant: diagnosis clusters are built from at least one spike point before anchor selection.
        .expect("cluster must have points");

    ClusterAnchor {
        task: fallback.task,
        class: fallback.class,
        comm: fallback.comm.clone(),
        latency_ns: fallback.latency_ns,
        kind: ClusterAnchorKind::Other,
    }
}
