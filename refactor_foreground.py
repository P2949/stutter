import os
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()

    # Field accesses on event/snapshot
    # Warning: this might match other things with similar names, but we'll try to be safe.
    # event.pid -> event.decision.target.as_ref().and_then(|t| t.pid)
    for var in ["event", "snapshot", "final_event", "foreground", "last", "stale"]:
        content = re.sub(rf'\b{var}\.pid\b', rf'{var}.decision.target.as_ref().and_then(|t| t.pid)', content)
        content = re.sub(rf'\b{var}\.app_id\b', rf'{var}.decision.target.as_ref().and_then(|t| t.app_id.clone())', content)
        content = re.sub(rf'\b{var}\.class\b', rf'{var}.decision.target.as_ref().and_then(|t| t.class.clone())', content)
        content = re.sub(rf'\b{var}\.title\b', rf'{var}.decision.target.as_ref().and_then(|t| t.title.clone())', content)
        content = re.sub(rf'\b{var}\.window_id\b', rf'{var}.decision.target.as_ref().and_then(|t| t.window_id.clone())', content)
        content = re.sub(rf'\b{var}\.workspace\b', rf'{var}.decision.target.as_ref().and_then(|t| t.workspace.clone())', content)
        content = re.sub(rf'\b{var}\.confidence\b', rf'{var}.decision.confidence', content)
        # Note: we will change reasons access later, for now we can leave reason or fix it if it's used.
        content = re.sub(rf'\b{var}\.reason\b', rf'{var}.decision.reasons.first().map(|r| r.reason.clone()).unwrap_or_default()', content)

    with open(filepath, 'w') as f:
        f.write(content)


src_dir = "stutter/src"
for root, dirs, files in os.walk(src_dir):
    for file in files:
        if file.endswith(".rs"):
            process_file(os.path.join(root, file))

