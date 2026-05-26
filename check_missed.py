import os

found = set()
mapping = {
    "startup": ["new"],
    "probes": [
        "handle_probe_drain", "handle_scx_tick", "handle_hwmon_tick",
        "handle_wayland_presentation_tick", "handle_dmabuf_tick",
        "handle_wayland_presentation_log_tick", "handle_dmabuf_log_tick",
        "normalize_wayland_presentation_event", "normalize_dmabuf_event"
    ],
    "targets": [
        "refresh_tasks", "refresh_tasks_and_emit_snapshot",
        "handle_target_tick", "handle_tree_tick", "handle_watch_tick"
    ],
    "exporters": [
        "emit", "dispatch_monitor_event", "handle_summary_tick",
        "handle_summary_context_tick", "handle_frame_tick",
        "handle_live_spike", "handle_telemetry_tick",
        "handle_focus_context_tick", "handle_foreground_context_tick"
    ],
    "shutdown": [
        "finalize", "handle_ctrl_c_stop", "handle_max_duration_stop",
        "handle_remote_stop", "handle_epoch_tick"
    ],
    "display": [
        "handle_ui_tick", "handle_tui_event"
    ]
}

for funcs in mapping.values():
    for f in funcs:
        found.add(f)

extracted = [
 "new",
 "emit",
 "refresh_tasks_and_emit_snapshot",
 "dispatch_monitor_event",
 "handle_ctrl_c_stop",
 "handle_max_duration_stop",
 "handle_remote_stop",
 "handle_epoch_tick",
 "handle_target_tick",
 "handle_focus_context_tick",
 "handle_foreground_context_tick",
 "handle_summary_context_tick",
 "handle_probe_drain",
 "handle_frame_tick",
 "handle_wayland_presentation_tick",
 "handle_dmabuf_tick",
 "normalize_wayland_presentation_event",
 "normalize_dmabuf_event",
 "handle_wayland_presentation_log_tick",
 "handle_dmabuf_log_tick",
 "handle_telemetry_tick",
 "handle_ui_tick",
 "handle_tui_event",
 "handle_summary_tick",
 "handle_tree_tick",
 "handle_watch_tick",
 "handle_scx_tick",
 "handle_hwmon_tick",
 "handle_live_spike",
 "refresh_tasks",
 "finalize",
]

for ext in extracted:
    if ext not in found:
        print("MISSED:", ext)

