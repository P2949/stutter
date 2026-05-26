import re
import os

with open("stutter/src/session/monitor_session.rs", "r") as f:
    content = f.read()

impl_blocks = []
standalone_funcs = []

lines = content.splitlines()
in_impl = False
current_fn = None
brace_depth = 0
has_entered_body = False
fn_lines = []
prelude_lines = []

i = 0
while i < len(lines):
    line = lines[i]
    if line.startswith("impl MonitorSession {"):
        in_impl = True
        i += 1
        continue

    if not in_impl:
        if line.startswith("fn ") or line.startswith("pub fn "):
            current_fn = re.search(r'fn (\w+)', line).group(1)
            fn_lines.append(line)
            brace_depth += line.count('{') - line.count('}')
            if '{' in line:
                has_entered_body = True
            if has_entered_body and brace_depth == 0:
                standalone_funcs.append({"name": current_fn, "code": "\n".join(fn_lines)})
                current_fn = None
                has_entered_body = False
                fn_lines = []
        elif current_fn:
            fn_lines.append(line)
            brace_depth += line.count('{') - line.count('}')
            if '{' in line:
                has_entered_body = True
            if has_entered_body and brace_depth == 0:
                standalone_funcs.append({"name": current_fn, "code": "\n".join(fn_lines)})
                current_fn = None
                has_entered_body = False
                fn_lines = []
        else:
            prelude_lines.append(line)
        i += 1
        continue

    if in_impl and line == "}":
        in_impl = False
        i += 1
        continue

    if ("fn " in line) and not current_fn and ("(" in line or "<" in line):
        fn_lines.append(line)
        brace_depth += line.count('{') - line.count('}')
        if '{' in line:
            has_entered_body = True
        match = re.search(r'fn (\w+)', line)
        if match:
            current_fn = match.group(1)
        else:
            current_fn = "unknown"
    elif current_fn:
        fn_lines.append(line)
        brace_depth += line.count('{') - line.count('}')
        if '{' in line:
            has_entered_body = True
        if has_entered_body and brace_depth == 0:
            impl_blocks.append({"name": current_fn, "code": "\n".join(fn_lines)})
            current_fn = None
            has_entered_body = False
            fn_lines = []
    i += 1

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

os.makedirs("stutter/src/session/monitor_session_split", exist_ok=True)

for mod, funcs in mapping.items():
    code = "use super::*;\n\nimpl MonitorSession {\n"
    for block in impl_blocks:
        if block["name"] in funcs:
            code += "    " + block["code"].replace("\n", "\n    ") + "\n\n"
    code += "}\n"
    
    if mod == "display":
        for block in standalone_funcs:
            if block["name"] == "display_driver_from_source":
                code += "\n" + block["code"] + "\n"
    
    with open(f"stutter/src/session/monitor_session_split/{mod}.rs", "w") as f:
        f.write(code)

with open("stutter/src/session/monitor_session_split/mod.rs", "w") as f:
    f.write("\n".join(prelude_lines) + "\n")
    f.write("mod startup;\nmod probes;\nmod targets;\nmod exporters;\nmod shutdown;\nmod display;\nmod event_loop;\n\n")

