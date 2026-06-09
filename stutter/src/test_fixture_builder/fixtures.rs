//! Fixture constructor namespace split by source and helper ownership.

use super::*;

mod builders;
mod display_path;
mod real;
mod real_matrix;
mod synthetic;

pub(super) use builders::renamed_fixture;
pub(super) use display_path::{
    direct_gpu_clean_fixture, dmabuf_modifier_mismatch_fixture, missing_evidence_unknown_fixture,
    uhd630_composited_blitter_fixture, uhd630_cross_gpu_fence_wait_fixture,
    uhd630_kms_delay_fixture, wayland_zero_copy_good_fixture,
};
pub(super) use real::{
    real_amd_kwin_overlay_false_positive_fixture, real_block_io_overlap_fixture,
    real_clean_baseline_fixture, real_community_rules_classification_fixture,
    real_compositor_scheduler_delay_fixture, real_foreground_window_fixture,
    real_game_thread_scheduler_delay_fixture, real_gpu_bound_looking_fixture,
    real_intel_gnome_frame_jitter_false_positive_fixture, real_irq_overlap_fixture,
    real_known_miss_block_io_tail_fixture, real_known_miss_short_irq_overlap_fixture,
    real_known_miss_sparse_gpu_bound_fixture, real_truncated_low_quality_fixture,
};
pub(super) use real_matrix::{
    real_amd_gamescope_gpu_bound_fixture, real_amd_hyprland_clean_fixture,
    real_intel_kwin_cpu_bound_fixture, real_intel_sway_compositor_delay_fixture,
    real_nvidia_gnome_false_positive_fixture, real_nvidia_kwin_irq_overlap_fixture,
};
pub(super) use synthetic::{
    block_io_stall_fixture, clean_run_fixture, community_rules_classification_fixture,
    compositor_scheduler_delay_fixture, cpu_pressure_fixture, foreground_window_fixture,
    game_scheduler_pressure_fixture, game_thread_scheduler_delay_fixture,
    gpu_bound_clean_cpu_fixture, irq_heavy_fixture, old_schema_warning_fixture,
    public_clean_baseline_fixture, public_game_thread_scheduler_delay_fixture,
    public_low_quality_truncated_fixture, reused_tid_no_contamination_fixture,
    truncated_drop_counters_fixture,
};
