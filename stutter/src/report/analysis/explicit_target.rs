//! The process tree a recording names explicitly (`--tree-pid`, `--pid`) is the workload under
//! analysis. The recorder classifies tasks from names and paths, so an arbitrary target that is
//! not a known game executable lands in a generic class (`Service`, `Helper`, `Render`,
//! `Unknown`) and its scheduler delays were never considered as the target's own. Tasks of an
//! explicitly named process whose class is only such a generic fallback are treated as the
//! target. Infrastructure and specialised classes (compositor, audio, browser, kernel, build
//! tools, ...) are never reclassified: naming a process does not make its helpers a game.
use crate::{process_tree::TaskClass, spike::SpikeCluster};

fn is_generic_fallback(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Service | TaskClass::Helper | TaskClass::Render | TaskClass::Unknown
    )
}

/// Reclassifies generic-class points of the explicitly named processes as the target.
pub(crate) fn promote_explicit_targets(clusters: &mut [SpikeCluster], explicit_pids: &[u32]) {
    if explicit_pids.is_empty() {
        return;
    }
    for point in clusters.iter_mut().flat_map(|c| c.points.iter_mut()) {
        if point
            .process_pid
            .is_some_and(|pid| explicit_pids.contains(&pid))
            && is_generic_fallback(point.class)
        {
            point.class = TaskClass::Game;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spike::SpikePoint;

    fn point(pid: u32, class: TaskClass) -> SpikePoint {
        SpikePoint {
            process_pid: Some(pid),
            class,
            latency_ns: 5_000_000,
            ..SpikePoint::default()
        }
    }

    #[test]
    fn only_generic_tasks_of_a_named_process_become_the_target() {
        let mut clusters = vec![SpikeCluster {
            points: vec![
                point(10, TaskClass::Service),
                point(10, TaskClass::Unknown),
                point(10, TaskClass::Compositor),
                point(10, TaskClass::AudioRealtime),
                point(10, TaskClass::KernelThread),
                point(11, TaskClass::Service),
            ],
            ..SpikeCluster::default()
        }];
        promote_explicit_targets(&mut clusters, &[10]);
        let classes: Vec<TaskClass> = clusters[0].points.iter().map(|p| p.class).collect();
        assert_eq!(
            classes,
            vec![
                TaskClass::Game,
                TaskClass::Game,
                TaskClass::Compositor,
                TaskClass::AudioRealtime,
                TaskClass::KernelThread,
                TaskClass::Service,
            ]
        );
    }

    #[test]
    fn without_a_named_target_nothing_changes() {
        let mut clusters = vec![SpikeCluster {
            points: vec![point(10, TaskClass::Service)],
            ..SpikeCluster::default()
        }];
        promote_explicit_targets(&mut clusters, &[]);
        assert_eq!(clusters[0].points[0].class, TaskClass::Service);
    }
}
