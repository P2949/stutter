use super::helpers::{flatten_core_cpus, same_core};
use crate::topology::{CoreInfo, TopologyModel, cpu_mask_to_vec, cpus_to_mask, sorted_unique};

#[derive(Clone, Debug)]
pub(crate) struct CandidateCpuLayout {
    pub(crate) online_mask: crate::affinity::CpuMask,
    pub(crate) render_mask: crate::affinity::CpuMask,
    pub(crate) worker_mask: crate::affinity::CpuMask,
    pub(crate) compositor_mask: crate::affinity::CpuMask,
    pub(crate) helper_mask: crate::affinity::CpuMask,
    pub(crate) wine_server_mask: crate::affinity::CpuMask,
    pub(crate) separate_game_mask: crate::affinity::CpuMask,
    pub(crate) separate_compositor_mask: crate::affinity::CpuMask,
    pub(crate) avoid_smt_render_mask: crate::affinity::CpuMask,
    pub(crate) avoid_smt_compositor_mask: crate::affinity::CpuMask,
    pub(crate) avoid_smt_worker_mask: crate::affinity::CpuMask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoreChoice {
    pub(crate) package_id: Option<u32>,
    pub(crate) core_id: Option<u32>,
    pub(crate) numa_node: Option<u32>,
    pub(crate) cpus: Vec<u32>,
    pub(crate) primary_cpu: u32,
    pub(crate) max_mhz: Option<u64>,
}

impl CandidateCpuLayout {
    pub(crate) fn from_topology(topology: &TopologyModel) -> Option<Self> {
        let online_cpus = topology_online_cpus(topology);
        if online_cpus.is_empty() {
            return None;
        }

        let online_mask = cpus_to_mask(&online_cpus)?;
        let cores = topology_core_choices(topology, &online_cpus);
        let render_core_count = if cores.len() >= 6 { 2 } else { 1 };

        let render_cores = cores
            .iter()
            .take(render_core_count)
            .cloned()
            .collect::<Vec<_>>();
        let render_primary_cpus = render_cores
            .iter()
            .map(|core| core.primary_cpu)
            .collect::<Vec<_>>();
        let render_all_cpus = flatten_core_cpus(&render_cores);

        let non_render_cores = cores
            .iter()
            .filter(|core| !render_cores.iter().any(|render| same_core(render, core)))
            .cloned()
            .collect::<Vec<_>>();

        let worker_cpus = if non_render_cores.is_empty() {
            online_cpus.clone()
        } else {
            flatten_core_cpus(&non_render_cores)
        };

        let compositor_core = non_render_cores
            .iter()
            .find(|core| core.cpus.iter().all(|cpu| !render_all_cpus.contains(cpu)))
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let compositor_cpus = vec![compositor_core.primary_cpu];

        let wine_core = non_render_cores
            .iter()
            .find(|core| !same_core(core, &compositor_core))
            .cloned()
            .or_else(|| Some(compositor_core.clone()))?;
        let wine_server_cpus = wine_core.cpus.clone();

        let separate_compositor_core = compositor_core.clone();
        let separate_compositor_cpus = separate_compositor_core.cpus.clone();
        let separate_game_cpus = cores
            .iter()
            .filter(|core| !same_core(core, &separate_compositor_core))
            .flat_map(|core| core.cpus.iter().copied())
            .collect::<Vec<_>>();
        let separate_game_cpus = if separate_game_cpus.is_empty() {
            online_cpus.clone()
        } else {
            separate_game_cpus
        };

        let render_sibling_set = topology
            .smt_siblings
            .get(render_primary_cpus.first().unwrap_or(&online_cpus[0]))
            .cloned()
            .unwrap_or_else(|| render_all_cpus.clone());

        let avoid_smt_compositor_core = non_render_cores
            .iter()
            .find(|core| {
                core.cpus
                    .iter()
                    .all(|cpu| !render_sibling_set.contains(cpu))
            })
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let avoid_smt_worker_cpus = online_cpus
            .iter()
            .copied()
            .filter(|cpu| {
                !render_sibling_set.contains(cpu) && !avoid_smt_compositor_core.cpus.contains(cpu)
            })
            .collect::<Vec<_>>();
        let avoid_smt_worker_cpus = if avoid_smt_worker_cpus.is_empty() {
            worker_cpus.clone()
        } else {
            avoid_smt_worker_cpus
        };

        Some(Self {
            online_mask,
            render_mask: cpus_to_mask(&render_primary_cpus)?,
            worker_mask: cpus_to_mask(&worker_cpus)?,
            compositor_mask: cpus_to_mask(&compositor_cpus)?,
            helper_mask: cpus_to_mask(&worker_cpus)?,
            wine_server_mask: cpus_to_mask(&wine_server_cpus)?,
            separate_game_mask: cpus_to_mask(&separate_game_cpus)?,
            separate_compositor_mask: cpus_to_mask(&separate_compositor_cpus)?,
            avoid_smt_render_mask: cpus_to_mask(&render_primary_cpus)?,
            avoid_smt_compositor_mask: cpus_to_mask(&[avoid_smt_compositor_core.primary_cpu])?,
            avoid_smt_worker_mask: cpus_to_mask(&avoid_smt_worker_cpus)?,
        })
    }
}

pub(crate) fn topology_online_cpus(topology: &TopologyModel) -> Vec<u32> {
    let online = topology.online_cpu_ids();
    if online.is_empty() {
        cpu_mask_to_vec(&topology.online_cpus)
    } else {
        online
    }
}

pub(crate) fn topology_core_choices(
    topology: &TopologyModel,
    online_cpus: &[u32],
) -> Vec<CoreChoice> {
    let mut choices = topology
        .cores
        .iter()
        .filter(|core| core.is_online)
        .filter_map(|core| core_choice_from_core(core, online_cpus))
        .collect::<Vec<_>>();

    if choices.is_empty() {
        choices = online_cpus
            .iter()
            .copied()
            .map(|cpu| CoreChoice {
                package_id: None,
                core_id: Some(cpu),
                numa_node: None,
                cpus: vec![cpu],
                primary_cpu: cpu,
                max_mhz: topology.cpu_info(cpu).and_then(|info| info.max_mhz),
            })
            .collect();
    }

    choices.sort_by(|left, right| {
        right
            .max_mhz
            .unwrap_or(0)
            .cmp(&left.max_mhz.unwrap_or(0))
            .then_with(|| left.package_id.cmp(&right.package_id))
            .then_with(|| left.numa_node.cmp(&right.numa_node))
            .then_with(|| left.core_id.cmp(&right.core_id))
            .then_with(|| left.primary_cpu.cmp(&right.primary_cpu))
    });

    choices
}

pub(crate) fn core_choice_from_core(core: &CoreInfo, online_cpus: &[u32]) -> Option<CoreChoice> {
    let cpus = sorted_unique(
        core.cpus
            .iter()
            .copied()
            .filter(|cpu| online_cpus.contains(cpu))
            .collect(),
    );
    let primary_cpu = cpus.first().copied()?;

    Some(CoreChoice {
        package_id: core.package_id,
        core_id: core.core_id,
        numa_node: core.numa_node,
        cpus,
        primary_cpu,
        max_mhz: core.max_mhz,
    })
}
