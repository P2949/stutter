//! `stutter topology-check`: an advisory cross-check of the native topology against hwloc.
use std::path::PathBuf;

#[derive(Debug)]
pub struct TopologyCheckCommandInput {
    pub hwloc_xml: PathBuf,
}

pub fn run_topology_check_command(input: TopologyCheckCommandInput) -> anyhow::Result<()> {
    use crate::{topology::TopologyModel, topology_hwloc, topology_v2::TopologyV2};
    let metadata = std::fs::metadata(&input.hwloc_xml)?;
    anyhow::ensure!(
        metadata.len() <= topology_hwloc::MAX_XML_BYTES as u64,
        "the export is over the size limit"
    );
    let xml = std::fs::read_to_string(&input.hwloc_xml)?;
    let objects = topology_hwloc::parse_objects(&xml)?;
    let base = TopologyModel::read()?;
    let native = TopologyV2::read_at(
        std::path::Path::new("/sys/devices/system"),
        std::path::Path::new("/sys/class"),
        &base,
        base.online_cpu_ids(),
    );
    let check = topology_hwloc::cross_check(&native, &objects);
    if check.agrees {
        println!(
            "topology-check: the native topology agrees with hwloc ({} object(s) read)",
            objects.len()
        );
        return Ok(());
    }
    for difference in &check.differences {
        println!("difference: {difference}");
    }
    anyhow::bail!(
        "{} difference(s): the native topology is unchanged; this check is advisory",
        check.differences.len()
    )
}
