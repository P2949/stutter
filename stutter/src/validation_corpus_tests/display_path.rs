use super::assertions::*;

#[test]
fn validation_corpus_direct_gpu_clean_display_path() {
    let analysis = assert_fixture_from_metadata("direct_gpu_clean");

    assert_eq!(analysis.display_path_diagnosis.is_cross_gpu, Some(false));
    assert_eq!(analysis.direct_scanout.status, "yes");
    assert_eq!(analysis.display_path_diagnosis.verdict, "low");
    assert!(
        analysis.display_path_diagnosis.suspicion_score < 0.25,
        "unexpected display-path suspicion: {:?}",
        analysis.display_path_diagnosis
    );
}

#[test]
fn validation_corpus_uhd630_cross_gpu_fence_wait() {
    let analysis = assert_fixture_from_metadata("uhd630_cross_gpu_fence_wait");
    let diagnosis = &analysis.display_path_diagnosis;

    assert_eq!(diagnosis.is_cross_gpu, Some(true));
    assert_eq!(diagnosis.confidence, "high");
    assert_eq!(diagnosis.cross_gpu_fence.high_confidence_count, 1);
    assert_eq!(diagnosis.fence_component.status, "likely");
    assert!(diagnosis.suspicion_score >= 0.75);
}

#[test]
fn validation_corpus_uhd630_composited_blitter() {
    let analysis = assert_fixture_from_metadata("uhd630_composited_blitter");
    let diagnosis = &analysis.display_path_diagnosis;

    assert_eq!(analysis.direct_scanout.status, "no");
    assert_eq!(diagnosis.compositor_component.status, "likely");
    assert!(
        diagnosis
            .gpu_engine_activity
            .as_ref()
            .is_some_and(|activity| activity.igpu_blitter_activity_near_outliers > 0)
    );
}

#[test]
fn validation_corpus_uhd630_kms_delay() {
    let analysis = assert_fixture_from_metadata("uhd630_kms_delay");

    assert_eq!(analysis.display_path_diagnosis.is_cross_gpu, Some(true));
    assert_eq!(analysis.kms_timing.p99_flip_ms, Some(3.2));
    assert_eq!(
        analysis.display_path_diagnosis.kms_component.status,
        "candidate"
    );
}

#[test]
fn validation_corpus_wayland_zero_copy_good() {
    let analysis = assert_fixture_from_metadata("wayland_zero_copy_good");

    assert_eq!(analysis.direct_scanout.status, "yes");
    assert_eq!(analysis.direct_scanout.confidence, "medium");
    assert_eq!(analysis.display_path_diagnosis.verdict, "low");
}

#[test]
fn validation_corpus_dmabuf_modifier_mismatch() {
    let analysis = assert_fixture_from_metadata("dmabuf_modifier_mismatch");
    let dmabuf = analysis
        .display_path_diagnosis
        .dmabuf_path
        .as_ref()
        .expect("expected DMABUF path summary");

    assert_eq!(dmabuf.modifier_mismatch_count, 1);
    assert_eq!(dmabuf.copy_required_count, 1);
    assert_eq!(dmabuf.cross_gpu_import_count, 1);
}
