//! A cross-boot comparison of boot environments and a resumable campaign ledger (package W14).
//!
//! A result measured on one kernel, preemption model or isolation set says nothing about another,
//! so blocks from different environments are never pooled. `compare_boots` names exactly what
//! differs; `CrossBootCampaign` keeps blocks grouped by the environment's context identity (the
//! hash without the generation, so a rebuild that changes nothing does not split a group), survives
//! a reboot as JSON, and analyses each group on its own. It is bookkeeping: it never changes boot
//! parameters, never reboots, and a campaign is resumed by loading it after the next boot.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

use crate::boot_environment::BootEnvironment;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootDifference {
    Kernel(String, String),
    Preemption,
    Isolation(&'static str),
    Microcode(Option<String>, Option<String>),
    SchedulerExt(Option<String>, Option<String>),
    /// The command line changed in a way not visible in the parsed isolation keys.
    CommandLine,
    /// Reported but not disqualifying: a rebuild with an identical kernel and settings.
    Generation(Option<String>, Option<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootComparison {
    pub differences: Vec<BootDifference>,
}

impl BootComparison {
    /// Results may be pooled or compared only when nothing but the generation differs.
    pub fn comparable(&self) -> bool {
        self.differences
            .iter()
            .all(|difference| matches!(difference, BootDifference::Generation(..)))
    }
}

pub fn compare_boots(a: &BootEnvironment, b: &BootEnvironment) -> BootComparison {
    let mut differences = Vec::new();
    if a.kernel_release != b.kernel_release {
        differences.push(BootDifference::Kernel(
            a.kernel_release.clone(),
            b.kernel_release.clone(),
        ));
    }
    if a.preempt != b.preempt {
        differences.push(BootDifference::Preemption);
    }
    for (name, left, right) in [
        ("nohz_full", &a.isolation.nohz_full, &b.isolation.nohz_full),
        ("rcu_nocbs", &a.isolation.rcu_nocbs, &b.isolation.rcu_nocbs),
        ("isolcpus", &a.isolation.isolcpus, &b.isolation.isolcpus),
        (
            "irqaffinity",
            &a.isolation.irqaffinity,
            &b.isolation.irqaffinity,
        ),
    ] {
        if left != right {
            differences.push(BootDifference::Isolation(name));
        }
    }
    if a.microcode != b.microcode {
        differences.push(BootDifference::Microcode(
            a.microcode.clone(),
            b.microcode.clone(),
        ));
    }
    if a.scheduler_ext_ops != b.scheduler_ext_ops {
        differences.push(BootDifference::SchedulerExt(
            a.scheduler_ext_ops.clone(),
            b.scheduler_ext_ops.clone(),
        ));
    }
    if a.cmdline_hash != b.cmdline_hash
        && !differences
            .iter()
            .any(|d| matches!(d, BootDifference::Isolation(_)))
    {
        differences.push(BootDifference::CommandLine);
    }
    if a.generation != b.generation {
        differences.push(BootDifference::Generation(
            a.generation.clone(),
            b.generation.clone(),
        ));
    }
    BootComparison { differences }
}

/// One recorded block: a labelled value from one boot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CampaignBlock {
    pub boot_id: String,
    pub context_hash: String,
    pub condition: String,
    pub block_id: u32,
    pub value: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CrossBootCampaign {
    pub campaign_id: String,
    pub blocks: Vec<CampaignBlock>,
}

impl CrossBootCampaign {
    pub fn new(campaign_id: &str) -> Self {
        Self {
            campaign_id: campaign_id.to_owned(),
            blocks: Vec::new(),
        }
    }

    /// Records a block measured under `environment`. A repeated (boot, condition, block) is
    /// refused so a resumed campaign never double counts.
    pub fn record(
        &mut self,
        environment: &BootEnvironment,
        condition: &str,
        block_id: u32,
        value: f64,
    ) -> anyhow::Result<()> {
        ensure!(value.is_finite(), "a block value must be finite");
        ensure!(
            !self.blocks.iter().any(|b| b.boot_id == environment.boot_id
                && b.condition == condition
                && b.block_id == block_id),
            "block {block_id} of {condition} was already recorded in boot {}",
            environment.boot_id
        );
        self.blocks.push(CampaignBlock {
            boot_id: environment.boot_id.clone(),
            context_hash: environment.context_hash.clone(),
            condition: condition.to_owned(),
            block_id,
            value,
        });
        Ok(())
    }

    /// Blocks grouped by environment context then condition. Groups are never merged: analysis
    /// runs inside one context only.
    pub fn by_context(&self) -> BTreeMap<String, BTreeMap<String, Vec<&CampaignBlock>>> {
        let mut groups: BTreeMap<String, BTreeMap<String, Vec<&CampaignBlock>>> = BTreeMap::new();
        for block in &self.blocks {
            groups
                .entry(block.context_hash.clone())
                .or_default()
                .entry(block.condition.clone())
                .or_default()
                .push(block);
        }
        groups
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path).context("cannot publish the campaign file")
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        serde_json::from_slice(&fs::read(path)?).context("not a cross-boot campaign file")
    }
}

#[cfg(test)]
#[path = "boot_campaign_tests.rs"]
mod tests;
