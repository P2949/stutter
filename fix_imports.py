import os

d = "stutter/src/autotune/planning/profile_candidates"

def prepend(file, text):
    path = os.path.join(d, file)
    with open(path, "r") as f:
        content = f.read()
    with open(path, "w") as f:
        f.write(text + "\n" + content)

prepend("helpers.rs", "use crate::profiles::ProfileRule;\nuse crate::process_tree::TaskClass;\nuse super::topology::CoreChoice;\n")
prepend("topology.rs", "use crate::topology::{CoreInfo, TopologyModel, cpu_mask_to_vec, cpus_to_mask, sorted_unique};\nuse super::helpers::{same_core, flatten_core_cpus};\n")
prepend("gaming.rs", "use crate::profiles::{Profile, ProfileRule};\nuse crate::process_tree::{CompiledPattern, TaskClass};\nuse super::topology::CandidateCpuLayout;\n")
prepend("validate.rs", "use crate::profiles::{Profile, ProfileRule};\nuse crate::topology::{TopologyModel, cpu_mask_to_vec};\nuse super::{GeneratedCpuSetPolicy};\nuse super::helpers::*;\n")
prepend("rules.rs", "use super::*;\nuse crate::topology::TopologyModel;\nuse crate::profiles::Profile;\nuse super::super::candidate::CandidateAction;\nuse std::collections::BTreeSet;\nuse super::topology::CandidateCpuLayout;\nuse super::gaming::*;\nuse super::validate::*;\nuse super::helpers::*;\n")

# mod.rs needs some re-exports and should probably remove the old top-level imports that are unused.
# Let's just prepend "pub use rules::*;" to mod.rs to export the functions.
prepend("mod.rs", "pub use rules::*;\n")

