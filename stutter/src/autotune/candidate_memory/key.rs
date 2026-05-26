use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::model::CandidateClassCount;
use crate::{
    autotune::{
        observation::AutotuneObservation, planning::candidate::CandidateAction,
        state::SituationKind,
    },
    focus::FocusGroupKind,
    process_tree::{TargetSnapshot, TaskClass},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct WorkloadIdentity(String);

impl WorkloadIdentity {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into().trim().to_owned();
        (!value.is_empty()).then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutableFingerprint {
    dev: u64,
    ino: u64,
}

impl ExecutableFingerprint {
    pub fn new(dev: u64, ino: u64) -> Self {
        Self { dev, ino }
    }

    pub fn from_parts(dev: Option<u64>, ino: Option<u64>) -> Option<Self> {
        Some(Self::new(dev?, ino?))
    }

    pub fn dev(&self) -> u64 {
        self.dev
    }

    pub fn ino(&self) -> u64 {
        self.ino
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CandidateMemoryKey(String);

impl CandidateMemoryKey {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into().trim().to_owned();
        (!value.is_empty()).then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CandidateIdentitySummary(String);

impl CandidateIdentitySummary {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into().trim().to_owned();
        (!value.is_empty()).then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateContextHashInput {
    pub workload_identity: Option<WorkloadIdentity>,
    #[serde(default)]
    pub workload_root_pid: Option<u32>,
    #[serde(default)]
    pub workload_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub workload_focus_kind: Option<FocusGroupKind>,
    pub target_executable: Option<ExecutableFingerprint>,
    pub cgroup_path: Option<String>,
    pub cpu_topology_signature: Option<String>,
    pub profile_name: String,
    pub situation: SituationKind,
    pub active_task_class_distribution: Vec<CandidateClassCount>,
}

impl CandidateContextHashInput {
    pub fn for_candidate(candidate: &CandidateAction) -> Self {
        Self {
            workload_identity: None,
            workload_root_pid: None,
            workload_starttime_ticks: None,
            workload_focus_kind: None,
            target_executable: None,
            cgroup_path: None,
            cpu_topology_signature: None,
            profile_name: candidate.candidate_name().to_owned(),
            situation: SituationKind::Unknown,
            active_task_class_distribution: Vec::new(),
        }
    }

    pub fn from_observation(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
    ) -> Self {
        Self {
            workload_identity: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| WorkloadIdentity::new(identity.stable_hash.clone())),
            workload_root_pid: observation
                .workload_identity
                .as_ref()
                .map(|identity| identity.root_pid),
            workload_starttime_ticks: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.process_starttime_ticks),
            workload_focus_kind: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.focus_kind),
            target_executable: observation.workload_identity.as_ref().and_then(|identity| {
                ExecutableFingerprint::from_parts(identity.exe_dev, identity.exe_ino)
            }),
            cgroup_path: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| normalize_optional_string(identity.cgroup_path.as_deref())),
            cpu_topology_signature: normalize_optional_string(
                cpu_topology_signature.or(observation.topology_signature.as_deref()),
            ),
            profile_name: candidate.candidate_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: class_distribution_from_workload_identity(observation),
        }
    }

    pub fn from_snapshot(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
        snapshot: &TargetSnapshot,
    ) -> Self {
        let (target_exe_dev, target_exe_ino) =
            target_executable_inode_from_snapshot(observation, snapshot);

        Self {
            workload_identity: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| WorkloadIdentity::new(identity.stable_hash.clone())),
            workload_root_pid: observation
                .workload_identity
                .as_ref()
                .map(|identity| identity.root_pid),
            workload_starttime_ticks: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.process_starttime_ticks),
            workload_focus_kind: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.focus_kind),
            target_executable: ExecutableFingerprint::from_parts(target_exe_dev, target_exe_ino),
            cgroup_path: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| normalize_optional_string(identity.cgroup_path.as_deref())),
            cpu_topology_signature: normalize_optional_string(
                cpu_topology_signature.or(observation.topology_signature.as_deref()),
            ),
            profile_name: candidate.candidate_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: class_distribution_from_snapshot(snapshot),
        }
    }

    pub fn normalized(mut self) -> Self {
        self.cpu_topology_signature = self
            .cpu_topology_signature
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.cgroup_path = self
            .cgroup_path
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        self.active_task_class_distribution =
            normalize_class_distribution(self.active_task_class_distribution);

        self
    }

    pub fn has_workload_identity(&self) -> bool {
        self.workload_identity.is_some()
    }

    pub fn context_key(&self) -> CandidateMemoryKey {
        CandidateMemoryKey::new(self.context_hash())
            // invariant: context hash generation must produce a non-empty key
            .expect("context hash generation must produce a non-empty key")
    }

    pub fn identity_summary(&self) -> CandidateIdentitySummary {
        CandidateIdentitySummary::new(self.context_identity_parts().join(" "))
            .unwrap_or_else(|| CandidateIdentitySummary("empty-context".to_owned()))
    }

    pub fn workload_identity_summary(&self) -> Option<CandidateIdentitySummary> {
        self.workload_identity.as_ref()?;
        CandidateIdentitySummary::new(self.workload_identity_parts().join(" "))
    }

    pub fn context_hash(&self) -> String {
        stable_hash_hex(&self.context_hash_parts())
    }

    fn context_hash_parts(&self) -> Vec<String> {
        let normalized = self.clone().normalized();
        let mut parts = Vec::new();

        parts.push(format!(
            "workload_hash={}",
            normalized
                .workload_identity
                .as_ref()
                .map(WorkloadIdentity::as_str)
                .unwrap_or("-")
        ));
        let executable = normalized.target_executable.as_ref();
        parts.push(format!(
            "target_exe={}:{}",
            executable
                .map(|fingerprint| fingerprint.dev().to_string())
                .unwrap_or_else(|| "-".to_owned()),
            executable
                .map(|fingerprint| fingerprint.ino().to_string())
                .unwrap_or_else(|| "-".to_owned())
        ));

        parts.push(format!(
            "cgroup={}",
            normalized.cgroup_path.as_deref().unwrap_or("-")
        ));
        parts.push(format!(
            "cpu_topology={}",
            normalized.cpu_topology_signature.as_deref().unwrap_or("-")
        ));
        parts.push(format!("profile={}", normalized.profile_name));
        parts.push(format!("situation={:?}", normalized.situation));

        for count in &normalized.active_task_class_distribution {
            parts.push(format!("class={}:{}", count.class.as_str(), count.count));
        }

        parts
    }

    fn context_identity_parts(&self) -> Vec<String> {
        let mut parts = self.workload_identity_parts();
        let normalized = self.clone().normalized();

        parts.push(format!(
            "cpu_topology={}",
            normalized.cpu_topology_signature.as_deref().unwrap_or("-")
        ));
        parts.push(format!("profile={}", normalized.profile_name));
        parts.push(format!("situation={:?}", normalized.situation));
        parts.push(format!(
            "classes={}",
            class_distribution_summary(&normalized.active_task_class_distribution)
        ));
        parts
    }

    fn workload_identity_parts(&self) -> Vec<String> {
        let normalized = self.clone().normalized();
        let executable = normalized.target_executable.as_ref();
        vec![
            format!(
                "workload_hash={}",
                normalized
                    .workload_identity
                    .as_ref()
                    .map(WorkloadIdentity::as_str)
                    .unwrap_or("-")
            ),
            format!(
                "root_pid={}",
                normalized
                    .workload_root_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ),
            format!(
                "starttime_ticks={}",
                normalized
                    .workload_starttime_ticks
                    .map(|ticks| ticks.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ),
            format!(
                "exe={}:{}",
                executable
                    .map(|fingerprint| fingerprint.dev().to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                executable
                    .map(|fingerprint| fingerprint.ino().to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ),
            format!(
                "cgroup={}",
                normalized.cgroup_path.as_deref().unwrap_or("-")
            ),
            format!(
                "focus={}",
                normalized
                    .workload_focus_kind
                    .map(|kind| format!("{kind:?}"))
                    .unwrap_or_else(|| "-".to_owned())
            ),
        ]
    }
}

fn class_distribution_from_workload_identity(
    observation: &AutotuneObservation,
) -> Vec<CandidateClassCount> {
    observation
        .workload_identity
        .as_ref()
        .map(|identity| {
            identity
                .class_distribution
                .iter()
                .filter_map(|(class, count)| {
                    TaskClass::from_str_opt(class).map(|class| CandidateClassCount {
                        class,
                        count: *count,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn target_executable_inode_from_snapshot(
    observation: &AutotuneObservation,
    snapshot: &TargetSnapshot,
) -> (Option<u64>, Option<u64>) {
    let Some(target_root_pid) = observation.target_root_pid else {
        return (None, None);
    };

    for task in snapshot.tasks.values() {
        if task.process_pid.as_u32() == target_root_pid || task.tid.as_u32() == target_root_pid {
            return (task.exe_dev, task.exe_ino);
        }
    }

    (None, None)
}

fn class_distribution_from_snapshot(snapshot: &TargetSnapshot) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for task in snapshot.tasks.values() {
        *counts.entry(task.class).or_default() += 1;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_class_distribution(
    distribution: Vec<CandidateClassCount>,
) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for item in distribution {
        if item.count == 0 {
            continue;
        }

        *counts.entry(item.class).or_default() += item.count;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn class_distribution_summary(distribution: &[CandidateClassCount]) -> String {
    if distribution.is_empty() {
        return "-".to_owned();
    }

    distribution
        .iter()
        .map(|count| format!("{}:{}", count.class.as_str(), count.count))
        .collect::<Vec<_>>()
        .join(",")
}

fn stable_hash_hex(parts: &[String]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;

    for part in parts {
        for byte in part.len().to_string().as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= b':' as u64;
        hash = hash.wrapping_mul(0x100000001b3);

        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}
