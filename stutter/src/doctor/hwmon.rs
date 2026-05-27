use std::collections::BTreeMap;
use crate::hwmon;
use super::model::{DoctorCheck, DoctorInput, DoctorStatus};
use super::utils::yes_no;

pub(crate) fn hwmon_check(input: &DoctorInput) -> DoctorCheck {
    let report = hwmon::probe_hwmon_with_options(
        input.hwmon_root.as_deref(),
        input.hwmon_drm_card.as_deref(),
        input.hwmon_render_node.as_deref(),
    );
    let mut details = BTreeMap::new();
    details.insert(
        "selected_root".to_owned(),
        report
            .selected_root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_owned()),
    );
    details.insert(
        "gpu_busy_percent".to_owned(),
        yes_no(report.gpu_busy_available),
    );
    details.insert("vram_used".to_owned(), yes_no(report.vram_used_available));
    details.insert("vram_total".to_owned(), yes_no(report.vram_total_available));
    details.insert("temp".to_owned(), yes_no(report.temp_available));
    details.insert("power".to_owned(), yes_no(report.power_available));
    details.insert(
        "nvidia_smi_fallback".to_owned(),
        yes_no(report.nvidia_fallback_available),
    );
    for (idx, warning) in report.warnings.iter().enumerate() {
        details.insert(format!("warning_{idx}"), warning.clone());
    }

    let status = if report.warnings.is_empty()
        && (report.gpu_busy_available || report.nvidia_fallback_available)
    {
        DoctorStatus::Pass
    } else {
        DoctorStatus::Warn
    };

    DoctorCheck {
        name: "hwmon".to_owned(),
        status,
        message: if matches!(status, DoctorStatus::Pass) {
            "GPU hwmon telemetry appears available".to_owned()
        } else {
            "GPU hwmon telemetry may be missing or partial".to_owned()
        },
        details,
    }
}
