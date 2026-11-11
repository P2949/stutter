//! The frozen, validated contract of one study.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use stutter::api::workload_progress::FixedWorkIdentity;

use crate::evidence_bundle::sha256::sha256_hex;

pub const MAX_BLOCKS: u32 = 12;
pub const MAX_CANDIDATES: usize = 6;
pub const MAX_WEIGHT: u32 = 10_000;
pub const BASELINE_WEIGHT: u32 = 100;

/// Where one role (the compile, or the background pressure generator) runs. A closed set: there is
/// no field for a path, a PID, a UID or a privileged target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Placement {
    /// Inherit the launching environment.
    Unrestricted,
    /// OwnedChildAffinity: a kernel cpulist applied to the child before it execs.
    Cpus(String),
    /// OwnedUserScopeCpuWeight: a transient user scope with this relative CPU weight.
    ScopeWeight(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    pub name: String,
    pub compile: Placement,
    pub hogs: Placement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub study_id: String,
    /// The fixed-work identity JSON (`xtask fixed-work --identity`).
    pub identity: PathBuf,
    pub hog_threads: u32,
    pub blocks: u32,
    /// Blocks 1..=training_blocks train the ranker; the rest are the untouched holdout.
    pub training_blocks: u32,
    pub arm_timeout_secs: u64,
    /// Practical-effect threshold in percent: the effect and equivalence boundary.
    pub practical_percent: f64,
    pub baseline: Arm,
    pub candidates: Vec<Arm>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Affinity,
    Scope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub block: u32,
    /// 0 is the baseline; candidate `i` is `i`.
    pub arm: usize,
    pub id: String,
}

pub fn load(path: &Path) -> anyhow::Result<Manifest> {
    serde_json::from_slice(&std::fs::read(path).context("read the manifest")?)
        .context("the manifest is not valid (unknown field or placement)")
}

pub fn parse_cpulist(text: &str) -> anyhow::Result<BTreeSet<u32>> {
    let mut cpus = BTreeSet::new();
    for part in text.trim().split(',') {
        let (low, high) = part.split_once('-').unwrap_or((part, part));
        let (low, high): (u32, u32) = (
            low.parse()
                .with_context(|| format!("bad cpulist {text:?}"))?,
            high.parse()
                .with_context(|| format!("bad cpulist {text:?}"))?,
        );
        ensure!(low <= high && high < 1024, "bad cpulist {text:?}");
        cpus.extend(low..=high);
    }
    ensure!(!cpus.is_empty(), "empty cpulist");
    Ok(cpus)
}

pub fn effective_cpus_of_status(status: &str) -> anyhow::Result<BTreeSet<u32>> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .context("no Cpus_allowed_list")?;
    parse_cpulist(line)
}

pub fn effective_cpus_of_self() -> anyhow::Result<BTreeSet<u32>> {
    effective_cpus_of_status(&std::fs::read_to_string("/proc/self/status")?)
}

fn plain_name(name: &str, max: usize) -> bool {
    !name.is_empty()
        && name.len() <= max
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl Arm {
    fn family(&self) -> anyhow::Result<Family> {
        let scoped = |p: &Placement| matches!(p, Placement::ScopeWeight(_));
        match (scoped(&self.compile), scoped(&self.hogs)) {
            (true, true) => Ok(Family::Scope),
            (false, false) => Ok(Family::Affinity),
            _ => bail!("arm {} mixes affinity and scope placements", self.name),
        }
    }

    fn check_placements(&self, allowed: &BTreeSet<u32>) -> anyhow::Result<()> {
        for placement in [&self.compile, &self.hogs] {
            match placement {
                Placement::Unrestricted => {}
                Placement::Cpus(list) => {
                    let cpus = parse_cpulist(list)?;
                    ensure!(
                        cpus.is_subset(allowed),
                        "arm {} names CPUs outside the effective set",
                        self.name
                    );
                }
                Placement::ScopeWeight(weight) => ensure!(
                    (1..=MAX_WEIGHT).contains(weight),
                    "arm {} has a weight outside 1..={MAX_WEIGHT}",
                    self.name
                ),
            }
        }
        Ok(())
    }
}

impl Manifest {
    pub fn digest(&self) -> String {
        sha256_hex(&serde_json::to_vec(self).unwrap_or_default())
    }

    pub fn arms(&self) -> Vec<&Arm> {
        std::iter::once(&self.baseline)
            .chain(&self.candidates)
            .collect()
    }

    /// Refuses everything that could reach outside the owned study, before anything is spawned.
    pub fn validate(&self, allowed: &BTreeSet<u32>) -> anyhow::Result<()> {
        ensure!(self.schema_version == 1, "unsupported schema version");
        ensure!(plain_name(&self.study_id, 64), "bad study id");
        ensure!(
            (2..=MAX_BLOCKS).contains(&self.blocks),
            "blocks out of range"
        );
        ensure!(
            (1..self.blocks).contains(&self.training_blocks),
            "training blocks must leave a holdout"
        );
        ensure!(
            (30..=900).contains(&self.arm_timeout_secs),
            "arm timeout out of range"
        );
        ensure!(
            self.practical_percent > 0.0 && self.practical_percent <= 50.0,
            "practical threshold out of range"
        );
        ensure!(
            (1..=allowed.len() as u32).contains(&self.hog_threads),
            "hog threads out of range"
        );
        ensure!(
            (1..=MAX_CANDIDATES).contains(&self.candidates.len()),
            "candidate count out of range"
        );
        ensure!(
            self.baseline.name == "baseline",
            "the baseline arm is named baseline"
        );
        let family = self.baseline.family()?;
        let neutral = match family {
            Family::Affinity => Placement::Unrestricted,
            Family::Scope => Placement::ScopeWeight(BASELINE_WEIGHT),
        };
        ensure!(
            self.baseline.compile == neutral && self.baseline.hogs == neutral,
            "the baseline must be neutral for its family"
        );
        let mut names = BTreeSet::new();
        for arm in self.arms() {
            ensure!(plain_name(&arm.name, 48), "bad arm name {:?}", arm.name);
            ensure!(
                names.insert(arm.name.as_str()),
                "duplicate arm {}",
                arm.name
            );
            ensure!(
                arm.family()? == family,
                "arm {} is another family",
                arm.name
            );
            arm.check_placements(allowed)?;
        }
        for arm in &self.candidates {
            ensure!(
                arm.compile != self.baseline.compile || arm.hogs != self.baseline.hogs,
                "candidate {} is a no-op",
                arm.name
            );
        }
        let identity: FixedWorkIdentity =
            serde_json::from_slice(&std::fs::read(&self.identity).context("read the identity")?)
                .context("the identity is not a fixed-work identity")?;
        identity.validate().map_err(anyhow::Error::msg)
    }

    /// Counterbalanced: the arm order rotates by block and reverses on even blocks (AB/BA).
    pub fn schedule(&self) -> Vec<Slot> {
        let arms = self.arms();
        let mut slots = Vec::new();
        for block in 1..=self.blocks {
            let mut order: Vec<usize> = (0..arms.len()).collect();
            order.rotate_left(((block - 1) as usize) % arms.len());
            if block % 2 == 0 {
                order.reverse();
            }
            slots.extend(order.into_iter().map(|arm| Slot {
                block,
                arm,
                id: format!("b{block:02}-{}", arms[arm].name),
            }));
        }
        slots
    }
}

/// Digest of everything that makes two timings comparable: running kernel, boot, effective CPU set,
/// sched_ext state and the exact fixed-work identity file.
pub fn context_digest(identity: &Path) -> anyhow::Result<String> {
    let read = |path: &str| {
        std::fs::read_to_string(path)
            .map_or_else(|_| "absent".to_owned(), |text| text.trim().to_owned())
    };
    let cpus = effective_cpus_of_self()?;
    let text = format!(
        "kernel={}\nboot={}\ncpus={cpus:?}\nsched_ext={}\nidentity={}\n",
        read("/proc/sys/kernel/osrelease"),
        read("/proc/sys/kernel/random/boot_id"),
        read("/sys/kernel/sched_ext/state"),
        sha256_hex(&std::fs::read(identity).context("read the identity")?),
    );
    Ok(sha256_hex(text.as_bytes()))
}
