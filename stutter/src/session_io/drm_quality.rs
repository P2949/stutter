use super::run_artifacts::RunArtifacts;
use crate::artifacts::{ArtifactKind, artifact_file_name};

pub(super) fn check_drm_fence_data_quality(artifacts: &mut RunArtifacts) {
    if !artifacts.session.config.drm_fence_latency {
        return;
    }

    let validation = &mut artifacts.validation;
    let events = &artifacts.drm_fence_events;
    let artifact_missing = validation
        .missing_optional_files
        .iter()
        .any(|file| file == artifact_file_name(ArtifactKind::DrmFenceEvents));

    if artifact_missing {
        validation.warnings.push(
            "DRM fence latency was requested but drm_fence_events.json is missing; tracepoints may have been unavailable"
                .to_owned(),
        );
    }

    if events.is_empty() {
        validation.warnings.push(
            "DRM fence latency was requested but no fence events were recorded; absence is not proof of no GPU wait"
                .to_owned(),
        );
    } else {
        if events
            .iter()
            .all(|event| event.event_kind != "wait_interval" || event.duration_ns.is_none())
        {
            validation.warnings.push(
                "DRM fence events contain only signal/marker evidence; wait duration attribution is low confidence"
                    .to_owned(),
            );
        }
        if events
            .iter()
            .any(|event| event.correlation_basis == "unknown")
        {
            validation.warnings.push(
                "DRM fence events include records without a stable context/seqno or timeline/seqno key"
                    .to_owned(),
            );
        }
        if events.iter().any(|event| {
            event.source == "unknown" || matches!(event.gpu_role.as_deref(), None | Some("unknown"))
        }) {
            validation.warnings.push(
                "DRM fence driver or GPU-role mapping is incomplete for some events".to_owned(),
            );
        }
    }

    if artifacts
        .session
        .config
        .drm_fence_render_card
        .as_deref()
        .is_none_or(str::is_empty)
        || artifacts
            .session
            .config
            .drm_fence_display_card
            .as_deref()
            .is_none_or(str::is_empty)
    {
        artifacts.validation.warnings.push(
            "DRM fence render/display cards were not both identified; cross-GPU attribution is approximate"
                .to_owned(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::DrmFenceEventRecord;

    #[test]
    fn drm_fence_requested_without_stable_waits_adds_validation_warnings() {
        let mut artifacts = RunArtifacts::default();
        artifacts.session.config.drm_fence_latency = true;
        artifacts.session.config.drm_fence_render_card = Some("card1".to_owned());
        artifacts.session.config.drm_fence_display_card = Some("card0".to_owned());
        artifacts.drm_fence_events.push(DrmFenceEventRecord {
            source: "amdgpu".to_owned(),
            event_kind: "signal".to_owned(),
            gpu_role: Some("render".to_owned()),
            correlation_basis: "unknown".to_owned(),
            ..Default::default()
        });

        check_drm_fence_data_quality(&mut artifacts);

        assert!(
            artifacts
                .validation
                .warnings
                .iter()
                .any(|warning| { warning.contains("only signal/marker evidence") })
        );
        assert!(
            artifacts
                .validation
                .warnings
                .iter()
                .any(|warning| { warning.contains("without a stable context/seqno") })
        );
    }

    #[test]
    fn drm_fence_requested_without_card_mapping_adds_warning() {
        let mut artifacts = RunArtifacts::default();
        artifacts.session.config.drm_fence_latency = true;

        check_drm_fence_data_quality(&mut artifacts);

        assert!(
            artifacts.validation.warnings.iter().any(|warning| {
                warning.contains("render/display cards were not both identified")
            })
        );
    }
}
