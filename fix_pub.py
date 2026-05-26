import glob
import re

for filepath in glob.glob("stutter/src/session/monitor_session/*.rs"):
    with open(filepath, "r") as f:
        content = f.read()
    
    # Replace anything like "    fn " with "    pub(crate) fn "
    content = re.sub(r'^([ \t]+)fn ', r'\1pub(crate) fn ', content, flags=re.MULTILINE)
    # Replace "    async fn " with "    pub(crate) async fn "
    content = re.sub(r'^([ \t]+)async fn ', r'\1pub(crate) async fn ', content, flags=re.MULTILINE)
    
    with open(filepath, "w") as f:
        f.write(content)

