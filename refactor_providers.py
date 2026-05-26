import os
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()

    # Providers often return:
    # return ForegroundWindowSnapshot {
    #     elapsed_ms,
    #     source: Some(...),
    #     status: ForegroundProviderStatus::...,
    #     confidence: 0.0,
    #     reason: ...,
    #     ..ForegroundWindowSnapshot::default()
    # };
    #
    # We want to change `confidence: 0.0` and `reason:` into `decision: ForegroundDecision { ... }`
    
    # 1. Error/Unavailable cases in providers where they pass confidence and reason
    pattern = re.compile(
        r'(source:\s*Some\([^)]+\),\s*)status:\s*(ForegroundProviderStatus::(?:Error|Unavailable|Unsupported)),\s*confidence:\s*(0\.0|0\.0f32),\s*reason:\s*([^,]+),',
        re.DOTALL
    )
    def repl(m):
        source = m.group(1)
        status = m.group(2)
        reason = m.group(4)
        return f'{source}status: {status},\ndecision: crate::foreground::model::ForegroundDecision {{\ntarget: None,\nconfidence: crate::foreground::model::Confidence::Zero,\nreasons: vec![crate::foreground::model::ForegroundReason {{ reason: {reason} }}],\nrejected_candidates: Vec::new(),\n}},'

    content = pattern.sub(repl, content)

    # 2. Hyprland available snapshot
    hyprland_avail = re.compile(
        r'ForegroundWindowSnapshot \{\s*elapsed_ms,\s*source:\s*Some\(ForegroundSource::Hyprland\),\s*status:\s*ForegroundProviderStatus::Available,\s*pid:\s*([^,]+),\s*app_id:\s*([^,]+),\s*class:\s*([^,]+),\s*title:\s*([^,]+),\s*window_id:\s*([^,]+),\s*workspace:\s*([^,]+),\s*confidence,\s*stale_ms:\s*None,\s*reason:\s*([^,]+),?\s*\}',
        re.DOTALL
    )
    def hypr_repl(m):
        pid, app_id, clazz, title, window_id, workspace, reason = m.groups()
        return f'''ForegroundWindowSnapshot {{
        elapsed_ms,
        source: Some(ForegroundSource::Hyprland),
        status: ForegroundProviderStatus::Available,
        decision: crate::foreground::model::ForegroundDecision {{
            target: Some(crate::foreground::model::ForegroundTarget {{
                pid: {pid},
                app_id: {app_id},
                class: {clazz},
                title: {title},
                window_id: {window_id},
                workspace: {workspace},
            }}),
            confidence: crate::foreground::model::Confidence::Medium, // TODO: map properly
            reasons: vec![crate::foreground::model::ForegroundReason {{ reason: {reason} }}],
            rejected_candidates: Vec::new(),
        }},
        stale_ms: None,
    }}'''
    content = hyprland_avail.sub(hypr_repl, content)

    # 3. Sway available snapshot
    sway_avail = re.compile(
        r'ForegroundWindowSnapshot \{\s*elapsed_ms,\s*source:\s*Some\(ForegroundSource::Sway\),\s*status:\s*ForegroundProviderStatus::Available,\s*pid:\s*([^,]+),\s*app_id:\s*([^,]+),\s*class:\s*([^,]+),\s*title:\s*([^,]+),\s*window_id:\s*([^,]+),\s*workspace:\s*([^,]+),\s*confidence,\s*stale_ms:\s*None,\s*reason:\s*([^,]+),?\s*\}',
        re.DOTALL
    )
    def sway_repl(m):
        pid, app_id, clazz, title, window_id, workspace, reason = m.groups()
        return f'''ForegroundWindowSnapshot {{
        elapsed_ms,
        source: Some(ForegroundSource::Sway),
        status: ForegroundProviderStatus::Available,
        decision: crate::foreground::model::ForegroundDecision {{
            target: Some(crate::foreground::model::ForegroundTarget {{
                pid: {pid},
                app_id: {app_id},
                class: {clazz},
                title: {title},
                window_id: {window_id},
                workspace: {workspace},
            }}),
            confidence: crate::foreground::model::Confidence::Medium, // TODO: map properly
            reasons: vec![crate::foreground::model::ForegroundReason {{ reason: {reason} }}],
            rejected_candidates: Vec::new(),
        }},
        stale_ms: None,
    }}'''
    content = sway_avail.sub(sway_repl, content)

    # 4. X11 available snapshot
    x11_avail = re.compile(
        r'ForegroundWindowSnapshot \{\s*elapsed_ms,\s*source:\s*Some\(ForegroundSource::X11\),\s*status:\s*ForegroundProviderStatus::Available,\s*pid:\s*([^,]+),\s*app_id:\s*([^,]+),\s*class:\s*([^,]+),\s*title:\s*([^,]+),\s*window_id:\s*([^,]+),\s*workspace:\s*([^,]+),\s*confidence,\s*stale_ms:\s*None,\s*reason:\s*([^,]+),?\s*\}',
        re.DOTALL
    )
    def x11_repl(m):
        pid, app_id, clazz, title, window_id, workspace, reason = m.groups()
        return f'''ForegroundWindowSnapshot {{
        elapsed_ms,
        source: Some(ForegroundSource::X11),
        status: ForegroundProviderStatus::Available,
        decision: crate::foreground::model::ForegroundDecision {{
            target: Some(crate::foreground::model::ForegroundTarget {{
                pid: {pid},
                app_id: {app_id},
                class: {clazz},
                title: {title},
                window_id: {window_id},
                workspace: {workspace},
            }}),
            confidence: crate::foreground::model::Confidence::Medium, // TODO: map properly
            reasons: vec![crate::foreground::model::ForegroundReason {{ reason: {reason} }}],
            rejected_candidates: Vec::new(),
        }},
        stale_ms: None,
    }}'''
    content = x11_avail.sub(x11_repl, content)

    with open(filepath, 'w') as f:
        f.write(content)

src_dir = "stutter/src/foreground/providers"
for root, dirs, files in os.walk(src_dir):
    for file in files:
        if file.endswith(".rs"):
            process_file(os.path.join(root, file))

