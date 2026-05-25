import re
from pathlib import Path

source_file = Path("stutter/src/report/model.rs")
target_file = Path("stutter-report/src/model.rs")

source_code = source_file.read_text()
target_code = target_file.read_text()

pure_types = [
    "SpikeDensityBucket",
    "SpikeClusterSource",
    "ArtifactsSummary",
    "KmsTimingSummary",
    "ScanoutWindowEstimate",
    "DrmFenceTimingSummary",
    "DrmFenceWaitSummary",
    "CrossGpuFenceSummary",
    "CrossGpuFenceCandidate",
    "WaylandPresentationSummary",
    "DirectScanoutSummary",
    "DmaBufPathSummary",
    "GpuEngineActivitySummary",
    "DisplayPathDiagnosisSummary",
    "DisplayPathComponent",
    "DataQualitySummary",
    "DataQualityLevel",
    "ForegroundReportSummary",
    "FocusReportSummary",
    "PressureTimelineSummary",
    "PressureTimelineCoverage",
    "PressurePeakWindow",
    "PressureKind",
    "PressureWindow",
    "RegressionMetric",
    "TextReportCorrelationSections",
    "TextReportCorrelationSection"
]

# We need to extract the definition of these types and any impl blocks.
# We will iterate through the lines and keep state.

lines = source_code.splitlines()

extracted_lines = []
remaining_lines = []

in_target = False
target_nesting = 0
current_target_lines = []

def starts_target(line):
    for t in pure_types:
        if re.search(r'pub(?:\(crate\))?\s+(?:struct|enum)\s+' + t + r'\b', line):
            return True
        if re.search(r'impl\s+' + t + r'\b', line):
            return True
    return False

# Also extract derive attributes immediately preceding the target
i = 0
while i < len(lines):
    line = lines[i]
    
    # Check for derives
    is_derive = line.startswith("#[derive")
    
    if is_derive and (i + 1 < len(lines) and starts_target(lines[i + 1])):
        extracted_lines.append(line)
        i += 1
        continue
    elif is_derive and (i + 2 < len(lines) and line.startswith("#[") and starts_target(lines[i + 2])):
        # handle multiple attributes
        pass # simplified for now
        
    if starts_target(line):
        in_target = True
        target_nesting = 0
        current_target_lines = []
    
    if in_target:
        current_target_lines.append(line)
        target_nesting += line.count('{') - line.count('}')
        if target_nesting == 0:
            extracted_lines.extend(current_target_lines)
            extracted_lines.append("")
            in_target = False
    else:
        # Check if the line is an attribute belonging to a target (naive)
        if line.startswith("#[") and (i + 1 < len(lines) and starts_target(lines[i+1])):
            extracted_lines.append(line)
        else:
            remaining_lines.append(line)
    i += 1

# Make sure we don't duplicate pub(crate) struct TextReportCorrelationSections if we need them pub
# Let's change pub(crate) to pub in the extracted lines for these generic model types
for i in range(len(extracted_lines)):
    extracted_lines[i] = extracted_lines[i].replace("pub(crate)", "pub")

target_code_lines = target_code.splitlines()

# find where to insert
insert_idx = len(target_code_lines)
for i, line in enumerate(target_code_lines):
    if line.startswith("#[cfg(test)]"):
        insert_idx = i
        break

new_target_code = "\n".join(target_code_lines[:insert_idx]) + "\n\n" + "\n".join(extracted_lines) + "\n\n" + "\n".join(target_code_lines[insert_idx:])
new_source_code = "\n".join(remaining_lines)

# Fix imports in target
imports = "use std::collections::BTreeMap;\nuse serde::{Deserialize, Serialize};\n"
new_target_code = new_target_code.replace("use serde::{Deserialize, Serialize};", imports, 1)

# we must remove `use serde::{Deserialize, Serialize};` duplicates
source_file.write_text(new_source_code)
target_file.write_text(new_target_code)

print("Extraction complete. Extracted", len(extracted_lines), "lines.")
