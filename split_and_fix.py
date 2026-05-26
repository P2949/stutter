import os
import re

source_file = "stutter/src/autotune/planning/profile_candidates.rs"
dest_dir = "stutter/src/autotune/planning/profile_candidates"
os.makedirs(dest_dir, exist_ok=True)

with open(source_file, "r") as f:
    lines = f.readlines()

def write_file(filename, start_line, end_line, prepend=""):
    content = "".join(lines[start_line-1:end_line])
    
    # replace fn -> pub(crate) fn except pub fn
    content = re.sub(r'^fn ', 'pub(crate) fn ', content, flags=re.MULTILINE)
    # replace struct -> pub(crate) struct
    content = re.sub(r'^struct ', 'pub(crate) struct ', content, flags=re.MULTILINE)
    
    # replace struct fields
    # only replace within the struct definition.
    if filename == "topology.rs":
        # match struct definitions and add pub(crate) to fields
        content = re.sub(r'^    (online_mask|render_mask|worker_mask|compositor_mask|helper_mask|wine_server_mask|separate_game_mask|separate_compositor_mask|avoid_smt_render_mask|avoid_smt_compositor_mask|avoid_smt_worker_mask|package_id|core_id|numa_node|cpus|primary_cpu|max_mhz): ', r'    pub(crate) \1: ', content, flags=re.MULTILINE)

    with open(os.path.join(dest_dir, filename), "w") as f:
        f.write(prepend + "\n" + content)

mod_prepend = "pub use rules::*;\n"
write_file("mod.rs", 1, 59, mod_prepend)

top_prepend = "use crate::topology::{CoreInfo, TopologyModel, cpu_mask_to_vec, cpus_to_mask, sorted_unique};\nuse super::helpers::{same_core, flatten_core_cpus};\n"
write_file("topology.rs", 152, 351, top_prepend)

rules_content = "".join(lines[59:151]) + "".join(lines[735:887])
rules_content = re.sub(r'^fn ', 'pub(crate) fn ', rules_content, flags=re.MULTILINE)
rules_prepend = "use super::*;\nuse crate::topology::TopologyModel;\nuse crate::profiles::Profile;\nuse super::super::candidate::CandidateAction;\nuse std::collections::BTreeSet;\nuse super::topology::CandidateCpuLayout;\nuse super::gaming::*;\nuse super::validate::*;\nuse super::helpers::*;\n"
with open(os.path.join(dest_dir, "rules.rs"), "w") as f:
    f.write(rules_prepend + "\n" + rules_content)

val_prepend = "use crate::profiles::{Profile, ProfileRule};\nuse crate::topology::{TopologyModel, cpu_mask_to_vec};\nuse super::{GeneratedCpuSetPolicy};\nuse super::helpers::*;\n"
write_file("validate.rs", 519, 683, val_prepend)

gam_prepend = "use crate::profiles::{Profile, ProfileRule};\nuse crate::process_tree::{CompiledPattern, TaskClass};\nuse super::topology::CandidateCpuLayout;\n"
write_file("gaming.rs", 368, 517, gam_prepend)

hlp_prepend = "use crate::topology::sorted_unique;\nuse crate::profiles::ProfileRule;\nuse crate::process_tree::TaskClass;\nuse super::topology::CoreChoice;\n"
hlp_content = "".join(lines[351:367]) + "".join(lines[684:734])
hlp_content = re.sub(r'^fn ', 'pub(crate) fn ', hlp_content, flags=re.MULTILINE)
with open(os.path.join(dest_dir, "helpers.rs"), "w") as f:
    f.write(hlp_prepend + "\n" + hlp_content)

os.remove(source_file)
