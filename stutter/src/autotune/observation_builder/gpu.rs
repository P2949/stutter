use std::path::Path;

use crate::autotune::{
    gpu_focus::{FocusGpuResolver, FocusGpuResolverInput, FocusGpuSource},
    objective::ObjectiveSignalQuality,
    observation::ActiveTaskSnapshot,
};

pub(crate) fn apply_focus_gpu_resolution(
    signals: &mut crate::autotune::objective::ObjectiveSignals,
    proc_root: &Path,
    target_root_pid: Option<u32>,
    active_tasks: &[ActiveTaskSnapshot],
    inventory: &crate::system_inventory::SystemInventory,
) {
    let target_pids = focus_gpu_target_pids(target_root_pid, active_tasks);
    let resolution = FocusGpuResolver::resolve(FocusGpuResolverInput {
        proc_root,
        target_pids: &target_pids,
        inventory,
        observed_render_node: signals.gpu_active_render_node.as_deref(),
        observed_drm_card: signals.gpu_drm_card.as_deref(),
        explicit_render_node: None,
        explicit_drm_card: None,
    });

    if resolution.source == FocusGpuSource::Unresolved {
        return;
    }

    signals.gpu_active_render_node = resolution.render_node;
    signals.gpu_drm_card = resolution.drm_card;
    signals.gpu_focus_confidence = Some(resolution.confidence);
    signals.gpu_focus_source = Some(resolution.source.as_str().to_owned());
    signals.signal_quality.gpu_active_render_node = match resolution.source {
        FocusGpuSource::TargetProcessFd
        | FocusGpuSource::ExplicitOverride
        | FocusGpuSource::GpuSample
        | FocusGpuSource::HwmonSelection => ObjectiveSignalQuality::Direct,
        FocusGpuSource::SingleGpuFallback => ObjectiveSignalQuality::Derived,
        FocusGpuSource::Unresolved => ObjectiveSignalQuality::Missing,
    };
}

pub(crate) fn focus_gpu_target_pids(
    target_root_pid: Option<u32>,
    active_tasks: &[ActiveTaskSnapshot],
) -> Vec<u32> {
    let mut pids = target_root_pid.into_iter().collect::<Vec<_>>();
    pids.extend(active_tasks.iter().map(|task| task.process_pid.as_u32()));
    pids.extend(active_tasks.iter().map(|task| task.tid.as_u32()));
    pids.sort_unstable();
    pids.dedup();
    pids
}
