use std::{collections::BTreeMap, fs};

use super::model::{DoctorCheck, DoctorStatus};

pub(crate) fn irq_selection_check(irqs: &[u32]) -> DoctorCheck {
    let mut details = BTreeMap::new();
    if !irqs.is_empty() {
        details.insert(
            "irqs".to_owned(),
            irqs.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
        return DoctorCheck {
            name: "irq_latency".to_owned(),
            status: DoctorStatus::Pass,
            message: "IRQ latency requested with explicit IRQ targets".to_owned(),
            details,
        };
    }

    let mut message =
        "no --irq supplied; inspect /proc/interrupts or use suggested GPU IRQ lines".to_owned();
    if let Ok(text) = fs::read_to_string("/proc/interrupts") {
        let suggestions = suggested_gpu_irq_lines_from_text(&text);
        for (idx, line) in suggestions.iter().take(8).enumerate() {
            details.insert(format!("suggested_irq_line_{idx}"), line.clone());
        }
        if suggestions.is_empty() {
            details.insert("suggestions".to_owned(), "none".to_owned());
        }
    } else {
        message.push_str("; /proc/interrupts was unreadable");
    }

    DoctorCheck {
        name: "irq_latency".to_owned(),
        status: DoctorStatus::Warn,
        message,
        details,
    }
}

pub fn suggested_gpu_irq_lines_from_text(text: &str) -> Vec<String> {
    const TERMS: &[&str] = &["amdgpu", "radeon", "nvidia", "i915", "xe", "drm", "gpu"];
    text.lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            TERMS.iter().any(|term| lower.contains(term))
        })
        .map(str::to_owned)
        .collect()
}
