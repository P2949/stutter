use std::{collections::BTreeMap, fs, path::PathBuf};

use super::super::{candidate::*, dry_run::*, profile_candidates::*};
use crate::{
    actions::SafetyClass,
    affinity::CpuMask,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
    topology::{CoreInfo, CpuInfo, TopologyModel},
};

pub(super) fn fake_topology_4c8t() -> TopologyModel {
    let online_cpus = CpuMask::parse("0-7").unwrap();

    TopologyModel {
        online_cpus,
        cpus: vec![
            fake_cpu(0, 0, 0, 0, 5000),
            fake_cpu(1, 1, 0, 0, 4900),
            fake_cpu(2, 2, 0, 0, 4800),
            fake_cpu(3, 3, 0, 0, 4700),
            fake_cpu(4, 0, 0, 0, 5000),
            fake_cpu(5, 1, 0, 0, 4900),
            fake_cpu(6, 2, 0, 0, 4800),
            fake_cpu(7, 3, 0, 0, 4700),
        ],
        cores: vec![
            fake_core(0, 0, 0, vec![0, 4], 5000),
            fake_core(1, 0, 0, vec![1, 5], 4900),
            fake_core(2, 0, 0, vec![2, 6], 4800),
            fake_core(3, 0, 0, vec![3, 7], 4700),
        ],
        smt_siblings: BTreeMap::from([
            (0, vec![0, 4]),
            (4, vec![0, 4]),
            (1, vec![1, 5]),
            (5, vec![1, 5]),
            (2, vec![2, 6]),
            (6, vec![2, 6]),
            (3, vec![3, 7]),
            (7, vec![3, 7]),
        ]),
        numa_nodes: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
        packages: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
    }
}

pub(super) fn fake_cpu(
    cpu: u32,
    core_id: u32,
    package_id: u32,
    numa_node: u32,
    max_mhz: u64,
) -> CpuInfo {
    CpuInfo {
        cpu,
        core_id: Some(core_id),
        package_id: Some(package_id),
        numa_node: Some(numa_node),
        max_mhz: Some(max_mhz),
        is_online: true,
    }
}

pub(super) fn fake_core(
    core_id: u32,
    package_id: u32,
    numa_node: u32,
    cpus: Vec<u32>,
    max_mhz: u64,
) -> CoreInfo {
    CoreInfo {
        core_id: Some(core_id),
        package_id: Some(package_id),
        numa_node: Some(numa_node),
        cpus,
        max_mhz: Some(max_mhz),
        is_online: true,
    }
}

pub(super) fn profile_by_name<'a>(profiles: &'a [Profile], name: &str) -> &'a Profile {
    profiles
        .iter()
        .find(|profile| profile.name == name)
        .unwrap_or_else(|| panic!("missing generated profile {name}"))
}

pub(super) fn first_rule_for_class(profile: &Profile, class: TaskClass) -> &ProfileRule {
    profile
        .rules
        .iter()
        .find(|rule| rule.match_class.contains(&class))
        .unwrap_or_else(|| panic!("missing rule for {class:?} in {}", profile.name))
}

pub(super) fn affinity(rule: &ProfileRule) -> &CpuMask {
    rule.affinity.as_ref().unwrap()
}

pub(super) fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    }
}

pub(super) fn status_for_profile(profile: &Profile) -> anyhow::Result<CandidateProfileStatus> {
    match profile.name.as_str() {
        "dry-run-fails" => anyhow::bail!("intentional dry-run failure"),
        "zero-match" => Ok(CandidateProfileStatus {
            matched_tasks: 0,
            dry_run_tasks: 0,
        }),
        _ => Ok(CandidateProfileStatus {
            matched_tasks: 2,
            dry_run_tasks: 1,
        }),
    }
}

pub(super) fn temp_candidate_plan_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-candidate-plan-{name}-{}",
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn eligible_record(name: &str, affected_tasks: usize) -> CandidateDryRunRecord {
    CandidateDryRunRecord {
        candidate_name: name.to_owned(),
        affected_tasks,
        warnings: Vec::new(),
        safety_class: SafetyClass::ReversibleLowRisk,
        eligible: true,
        reason: None,
    }
}

#[derive(Default)]
pub(super) struct FakeDryRunner {
    pub(super) dry_run_calls: usize,
    pub(super) apply_calls: usize,
}

impl CandidateDryRunner for FakeDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
        self.dry_run_calls += 1;
        eligible_record(candidate.profile_name(), 31)
    }
}

pub(super) fn dry_run_record(safety_class: SafetyClass) -> CandidateDryRunRecord {
    CandidateDryRunRecord {
        candidate_name: "game-main".to_owned(),
        affected_tasks: 4,
        warnings: Vec::new(),
        safety_class,
        eligible: true,
        reason: None,
    }
}
