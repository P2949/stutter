use std::collections::BTreeMap;

use super::*;

pub(crate) fn build_gpu_engine_activity_summary(
    engine_samples: &[crate::recorder::GpuEngineSample],
    frame_events: &[crate::recorder::FrameEvent],
) -> GpuEngineActivitySummary {
    let frame_outliers =
        identify_frame_spikes(frame_events, calculate_median_frametime(frame_events));
    let mut engine_counts = BTreeMap::new();
    let mut driver_counts = BTreeMap::new();
    let mut max_igpu_render_busy_percent: Option<f64> = None;
    let mut max_igpu_blitter_busy_percent: Option<f64> = None;
    let mut max_amdgpu_gfx_busy_percent: Option<f64> = None;
    let mut igpu_render_activity_near_outliers = 0usize;
    let mut igpu_blitter_activity_near_outliers = 0usize;
    let mut amdgpu_gfx_activity_near_outliers = 0usize;

    for sample in engine_samples {
        *engine_counts.entry(sample.engine.clone()).or_insert(0) += 1;
        if let Some(driver) = sample.driver.as_deref() {
            *driver_counts.entry(driver.to_owned()).or_insert(0) += 1;
        }

        let near_outlier = frame_outliers
            .iter()
            .any(|frame| elapsed_near(sample.elapsed_ms, frame.elapsed_ms, 20));
        let busy = sample.busy_percent.unwrap_or(0.0);
        if !near_outlier || busy < 10.0 {
            continue;
        }

        if gpu_engine_is_igpu(sample) && gpu_engine_is_render(&sample.engine) {
            igpu_render_activity_near_outliers += 1;
            max_igpu_render_busy_percent =
                Some(max_igpu_render_busy_percent.unwrap_or(0.0).max(busy));
        }
        if gpu_engine_is_igpu(sample) && gpu_engine_is_blitter(&sample.engine) {
            igpu_blitter_activity_near_outliers += 1;
            max_igpu_blitter_busy_percent =
                Some(max_igpu_blitter_busy_percent.unwrap_or(0.0).max(busy));
        }
        if gpu_engine_is_amdgpu(sample) && gpu_engine_is_gfx(&sample.engine) {
            amdgpu_gfx_activity_near_outliers += 1;
            max_amdgpu_gfx_busy_percent =
                Some(max_amdgpu_gfx_busy_percent.unwrap_or(0.0).max(busy));
        }
    }

    let mut notes = Vec::new();
    if engine_samples.is_empty() {
        notes.push("no GPU engine samples present".to_owned());
    } else if frame_outliers.is_empty() {
        notes.push("no frame outliers available for GPU engine proximity checks".to_owned());
    }
    if igpu_blitter_activity_near_outliers > 0 || igpu_render_activity_near_outliers > 0 {
        notes.push("iGPU render/blitter activity appeared near frame outliers".to_owned());
    }
    if amdgpu_gfx_activity_near_outliers > 0 {
        notes.push("AMD GFX activity appeared near frame outliers".to_owned());
    }
    let evidence_quality = if engine_samples.is_empty() {
        missing_evidence("no GPU engine samples present")
    } else {
        EvidenceQuality::Direct
    };

    GpuEngineActivitySummary {
        evidence_quality,
        sample_count: engine_samples.len(),
        active_sample_count: engine_samples
            .iter()
            .filter(|sample| sample.busy_percent.is_some_and(|busy| busy >= 10.0))
            .count(),
        engine_counts,
        driver_counts,
        igpu_render_activity_near_outliers,
        igpu_blitter_activity_near_outliers,
        amdgpu_gfx_activity_near_outliers,
        max_igpu_render_busy_percent,
        max_igpu_blitter_busy_percent,
        max_amdgpu_gfx_busy_percent,
        notes,
    }
}

fn gpu_engine_is_igpu(sample: &crate::recorder::GpuEngineSample) -> bool {
    matches!(sample.driver.as_deref(), Some("i915" | "xe"))
}

fn gpu_engine_is_amdgpu(sample: &crate::recorder::GpuEngineSample) -> bool {
    sample.driver.as_deref() == Some("amdgpu")
}

fn gpu_engine_is_render(engine: &str) -> bool {
    matches!(engine, "render" | "rcs" | "rcs0" | "3d")
}

fn gpu_engine_is_blitter(engine: &str) -> bool {
    matches!(engine, "blitter" | "bcs" | "bcs0")
}

fn gpu_engine_is_gfx(engine: &str) -> bool {
    matches!(engine, "gfx" | "gfx0" | "render" | "3d")
}
