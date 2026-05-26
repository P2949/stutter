import os
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
        
    orig_content = content

    if "session/ticks/foreground.rs" in filepath:
        content = content.replace("old.class.as_deref()", "old.decision.target.as_ref().and_then(|t| t.class.as_deref())")
        content = content.replace("new.class.as_deref()", "new.decision.target.as_ref().and_then(|t| t.class.as_deref())")
        content = content.replace("old.window_id.as_deref()", "old.decision.target.as_ref().and_then(|t| t.window_id.as_deref())")
        content = content.replace("new.window_id.as_deref()", "new.decision.target.as_ref().and_then(|t| t.window_id.as_deref())")
        content = content.replace("old.workspace.as_deref()", "old.decision.target.as_ref().and_then(|t| t.workspace.as_deref())")
        content = content.replace("new.workspace.as_deref()", "new.decision.target.as_ref().and_then(|t| t.workspace.as_deref())")

    if "commands/daemon/helpers.rs" in filepath:
        content = content.replace("event.decision.reasons.first().map(|r| r.reason.clone()).unwrap_or_default()", "event.reason.clone()")

    if "report/analysis/timing/dmabuf.rs" in filepath:
        content = content.replace("event.decision.reasons.first().map(|r| r.reason.clone()).unwrap_or_default()", "event.app_id.clone()")
        content = content.replace("event.decision.target.as_ref().and_then(|t| t.app_id.clone())", "event.app_id.clone()")

    if "report/analysis/timing/drm_fence.rs" in filepath:
        content = content.replace("event.decision.confidence.clone()", "event.confidence.clone()")

    if "report/render/text/summary.rs" in filepath:
        content = content.replace("foreground.decision.confidence", "foreground.confidence")

    if "tui/model.rs" in filepath:
        content = content.replace(".pid", ".decision.target.as_ref().and_then(|t| t.pid)")
        content = content.replace(".class", ".decision.target.as_ref().and_then(|t| t.class.clone())")
        content = content.replace(".decision.target.as_ref().and_then(|t| t.class.clone())_name", ".class_name")

    if "recorder/session/finalize.rs" in filepath:
        content = content.replace("event.decision.confidence)", "event.decision.confidence.as_f32())")

    if "report/analysis/foreground.rs" in filepath:
        # final_foreground_confidence is f32, but we need Confidence?
        # Actually session.core.final_foreground_confidence is Option<f32>.
        # We can just change `session.core.final_foreground_confidence` in metadata.rs to Option<Confidence> later.
        # But for now, let's just map it to f32 when setting, or map it back.
        # The easiest is: change event.decision.confidence to event.decision.confidence.as_f32()
        content = content.replace("event.decision.confidence)", "event.decision.confidence.as_f32())")
        content = content.replace("Some(event.decision.confidence)", "Some(event.decision.confidence.as_f32())")

    if content != orig_content:
        with open(filepath, 'w') as f:
            f.write(content)

src_dir = "stutter/src"
for root, dirs, files in os.walk(src_dir):
    for file in files:
        if file.endswith(".rs"):
            process_file(os.path.join(root, file))
