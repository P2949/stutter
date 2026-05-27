use std::collections::BTreeMap;

use super::{
    model::{DoctorCheck, DoctorStatus},
    utils::{available_unavailable, format_tracepoint_names, format_tracepoint_ref, yes_no},
};
use crate::drm_tracepoints;

pub(crate) fn kms_timing_check() -> DoctorCheck {
    let availability = drm_tracepoints::discover_kms_tracepoints_default();
    kms_timing_check_from_availability(availability)
}

pub(crate) fn kms_timing_check_from_availability(
    availability: drm_tracepoints::KmsTracepointAvailability,
) -> DoctorCheck {
    let mut details = BTreeMap::new();
    details.insert(
        "generic_drm_tracepoints".to_owned(),
        available_unavailable(!availability.generic_drm.is_empty()),
    );
    details.insert(
        "i915_pageflip_tracepoints".to_owned(),
        available_unavailable(!availability.i915.is_empty()),
    );
    details.insert(
        "amdgpu_pageflip_tracepoints".to_owned(),
        available_unavailable(!availability.amdgpu.is_empty()),
    );
    details.insert(
        "selected_provider".to_owned(),
        availability.selected_provider_name().to_owned(),
    );
    details.insert(
        "pageflip_request".to_owned(),
        format_tracepoint_ref(availability.pageflip_request.as_ref()),
    );
    details.insert(
        "pageflip_done".to_owned(),
        format_tracepoint_ref(availability.pageflip_done.as_ref()),
    );
    details.insert(
        "vblank_event".to_owned(),
        format_tracepoint_ref(availability.vblank_event.as_ref()),
    );
    details.insert(
        "atomic_commit".to_owned(),
        format_tracepoint_ref(availability.atomic_commit.as_ref()),
    );
    details.insert(
        "available_drm_tracepoints".to_owned(),
        format_tracepoint_names(&availability.generic_drm),
    );
    details.insert(
        "available_i915_tracepoints".to_owned(),
        format_tracepoint_names(&availability.i915),
    );
    details.insert(
        "available_amdgpu_tracepoints".to_owned(),
        format_tracepoint_names(&availability.amdgpu),
    );
    details.insert(
        "usable_crtc_id".to_owned(),
        yes_no(availability.has_usable_crtc_id()),
    );
    details.insert(
        "usable_sequence".to_owned(),
        yes_no(availability.has_usable_sequence()),
    );
    details.insert(
        "usable_timestamp".to_owned(),
        yes_no(availability.has_usable_timestamp()),
    );

    for (idx, warning) in availability.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }

    let usable = availability.has_selected_tracepoints();
    DoctorCheck {
        name: "kms_timing".to_owned(),
        status: if usable {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        message: if usable {
            "KMS timing tracepoints are usable with medium confidence".to_owned()
        } else {
            "KMS timing unavailable: no supported pageflip/vblank tracepoints found".to_owned()
        },
        details,
    }
}
