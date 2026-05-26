import os

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
        
    orig_content = content

    if "autotune/tui_panel.rs" in filepath:
        content = content.replace("event.decision.reasons.first().map(|r| r.reason.clone()).unwrap_or_default()", "event.reason.clone()")

    if "events.rs" in filepath:
        content = content.replace("event.decision.target.as_ref().and_then(|t| t.pid)", "event.pid")

    if "focus/foreground_match.rs" in filepath:
        content = content.replace("foreground.decision.confidence *", "foreground.decision.confidence.as_f32() *")

    if "focus/foreground_scoring.rs" in filepath:
        content = content.replace("foreground.decision.confidence <=", "foreground.decision.confidence.as_f32() <=")
        content = content.replace("fg.pid", "fg.decision.target.as_ref().and_then(|t| t.pid)")
        content = content.replace("fg.confidence", "fg.decision.confidence.as_f32()")
        # It's possible some were changed to fg.decision.confidence by previous steps or manual attempts
        content = content.replace("fg.decision.confidence.as_f32().as_f32()", "fg.decision.confidence.as_f32()")

    if "focus/snapshot.rs" in filepath:
        content = content.replace("fg.pid", "fg.decision.target.as_ref().and_then(|t| t.pid)")

    if "session/ticks/foreground.rs" in filepath:
        content = content.replace("old.pid", "old.decision.target.as_ref().and_then(|t| t.pid)")
        content = content.replace("new.pid", "new.decision.target.as_ref().and_then(|t| t.pid)")
        content = content.replace("old.app_id.as_deref()", "old.decision.target.as_ref().and_then(|t| t.app_id.as_deref())")
        content = content.replace("new.app_id.as_deref()", "new.decision.target.as_ref().and_then(|t| t.app_id.as_deref())")

    if "report/analysis/foreground.rs" in filepath:
        content = content.replace("event.decision.confidence.as_f32()", "event.confidence")
        # In case the previous step messed it up
        # Wait, if event is ForegroundEvent it has decision. If it's FocusEvent it has confidence.
        # Line 102 was: let confidence = final_event.map(|event| event.decision.confidence.as_f32());
        # where final_event is FocusEvent!
        # So we just replace `event.decision.confidence.as_f32()` with `event.confidence` and it will fix FocusEvent.
        # But wait, will it break ForegroundEvent which needs `.as_f32()`?
        # Let's be careful. The error was at line 102.
        # In report/analysis/foreground.rs, let's just do it directly.

    if "tui/model.rs" in filepath:
        content = content.replace("stats.decision.target.as_ref().and_then(|t| t.class.clone())", "stats.class.clone()")
        # wait, the code has `stats.decision.target.as_ref().and_then(|t| t.class)` etc.
        content = content.replace("stats.decision.target.as_ref().and_then(|t| t.class.clone())", "stats.class.clone()")
        content = content.replace("task.decision.target.as_ref().and_then(|t| t.pid)", "task.pid")
        content = content.replace("foreground.decision.confidence\n", "foreground.decision.confidence.as_f32()\n")
        content = content.replace("foreground.decision.confidence,", "foreground.decision.confidence.as_f32(),")

    if "report/analysis/timing/dmabuf.rs" in filepath:
        content = content.replace("event.app_id.clone().as_deref()", "event.app_id.as_deref()")

    if content != orig_content:
        with open(filepath, 'w') as f:
            f.write(content)

src_dir = "stutter/src"
for root, dirs, files in os.walk(src_dir):
    for file in files:
        if file.endswith(".rs"):
            process_file(os.path.join(root, file))
