//! Display timing report summaries.
//!
//! Owns KMS flip, DRM fence, Wayland presentation, DMABUF, GPU engine, and cross-GPU
//! summaries plus shared timing proximity and percentile helpers.

use super::*;

mod cross_gpu;
mod dmabuf;
mod drm_fence;
mod gpu_engine;
mod kms;
mod wayland;

pub(crate) use cross_gpu::build_cross_gpu_fence_summary;
pub(crate) use dmabuf::build_dmabuf_path_summary;
pub(crate) use drm_fence::build_drm_fence_timing_summary;
pub(crate) use gpu_engine::build_gpu_engine_activity_summary;
pub(crate) use kms::build_kms_timing_summary;
pub(crate) use wayland::{build_direct_scanout_summary, build_wayland_presentation_summary};

fn elapsed_near(left_ms: u64, right_ms: u64, window_ms: u64) -> bool {
    left_ms.abs_diff(right_ms) <= window_ms
}

fn optional_median_ms(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| {
        let mut values = values.to_vec();
        median_f64(&mut values)
    })
}

fn optional_percentile_ms(values: &[f64], percentile: f64) -> Option<f64> {
    (!values.is_empty()).then(|| {
        let mut values = values.to_vec();
        percentile_f64(&mut values, percentile)
    })
}

fn missing_evidence(reason: impl Into<String>) -> EvidenceQuality {
    EvidenceQuality::Missing {
        reason: reason.into(),
    }
}

fn approximate_evidence(reason: impl Into<String>) -> EvidenceQuality {
    EvidenceQuality::Approximate {
        reason: reason.into(),
    }
}
