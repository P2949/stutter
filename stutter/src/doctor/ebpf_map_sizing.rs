use crate::{
    doctor::{DoctorCheck, DoctorStatus},
    ebpf_loader,
};

pub(super) fn ebpf_map_sizing_check() -> DoctorCheck {
    let sizing = ebpf_loader::ebpf_map_sizing_report();
    let details = [
        (
            "locked_memory_limit_bytes",
            format_optional_u64(sizing.locked_memory_limit_bytes),
        ),
        (
            "available_memory_bytes",
            format_optional_u64(sizing.available_memory_bytes),
        ),
        (
            "events_ringbuf_bytes",
            sizing.events_ringbuf_bytes.to_string(),
        ),
        (
            "min_events_ringbuf_bytes",
            sizing.min_events_ringbuf_bytes.to_string(),
        ),
        (
            "max_events_ringbuf_bytes",
            sizing.max_events_ringbuf_bytes.to_string(),
        ),
        ("target_pids_max", sizing.target_pids_max.to_string()),
        (
            "wakeup_data_entries",
            sizing.wakeup_data_entries.to_string(),
        ),
        (
            "wakeup_data_map_entry_budget_bytes",
            sizing.wakeup_data_map_entry_budget_bytes.to_string(),
        ),
        (
            "min_wakeup_data_entries",
            sizing.min_wakeup_data_entries.to_string(),
        ),
        (
            "max_wakeup_data_entries",
            sizing.max_wakeup_data_entries.to_string(),
        ),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect();

    let status = if sizing.events_ringbuf_bytes <= 64 * 1024 || sizing.wakeup_data_entries <= 4096 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };

    DoctorCheck {
        name: "ebpf_map_sizing".to_owned(),
        status,
        message: if matches!(status, DoctorStatus::Pass) {
            "dynamic eBPF map sizing looks adequate".to_owned()
        } else {
            "dynamic eBPF map sizing is at the conservative minimum".to_owned()
        },
        details,
    }
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unlimited_or_unknown".to_owned())
}
