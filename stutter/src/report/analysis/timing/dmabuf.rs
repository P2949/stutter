use std::collections::BTreeMap;

use super::*;

pub(crate) fn build_dmabuf_path_summary(
    events: &[crate::recorder::DmaBufEventRecord],
) -> DmaBufPathSummary {
    let mut top_reasons = BTreeMap::new();
    for reason in events.iter().filter_map(|event| event.reason.as_deref()) {
        *top_reasons.entry(reason.to_owned()).or_insert(0) += 1;
    }

    let linear_count = events
        .iter()
        .filter(|event| {
            event.linear == Some(true)
                || event.modifier.as_deref().is_some_and(is_linear_modifier)
                || event
                    .modifier_name
                    .as_deref()
                    .is_some_and(is_linear_modifier)
        })
        .count();
    let scanout_capable_count = events
        .iter()
        .filter(|event| event.scanout_capable == Some(true))
        .count();
    let copy_required_count = events
        .iter()
        .filter(|event| event.copy_required == Some(true))
        .count();
    let modifier_mismatch_count = events
        .iter()
        .filter(|event| {
            event.reason.as_deref().is_some_and(|reason| {
                reason.contains("modifier_mismatch") || reason.contains("modifier mismatch")
            })
        })
        .count();
    let cross_gpu_import_count = events
        .iter()
        .filter(|event| dmabuf_cross_gpu(event))
        .count();

    let mut notes = Vec::new();
    if events.is_empty() {
        notes.push("no DMABUF path events present".to_owned());
    }
    if modifier_mismatch_count > 0 {
        notes.push("DMABUF log reported modifier mismatch candidates".to_owned());
    }
    if copy_required_count > 0 {
        notes.push("DMABUF log reported copy or linearization-required candidates".to_owned());
    }
    if cross_gpu_import_count > 0 {
        notes.push(
            "DMABUF allocation/import evidence crossed GPU or DRM-card boundaries".to_owned(),
        );
    }
    let evidence_quality = if events.is_empty() {
        missing_evidence("no DMABUF path events present")
    } else {
        EvidenceQuality::Direct
    };

    DmaBufPathSummary {
        evidence_quality,
        event_count: events.len(),
        linear_count,
        scanout_capable_count,
        copy_required_count,
        modifier_mismatch_count,
        cross_gpu_import_count,
        top_reasons,
        notes,
    }
}

fn is_linear_modifier(value: &str) -> bool {
    value.eq_ignore_ascii_case("linear")
        || value.eq_ignore_ascii_case("drm_format_modifier_linear")
        || value == "0"
}

fn dmabuf_cross_gpu(event: &crate::recorder::DmaBufEventRecord) -> bool {
    let cross_driver = event
        .allocation_driver
        .as_deref()
        .zip(event.import_driver.as_deref())
        .is_some_and(|(allocation, import)| !allocation.eq_ignore_ascii_case(import));
    let cross_card = event
        .allocation_card
        .as_deref()
        .zip(event.import_card.as_deref())
        .is_some_and(|(allocation, import)| allocation != import);
    cross_driver || cross_card
}
