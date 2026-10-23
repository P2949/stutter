//! Scenario definitions. A scenario is a *predeclared* experiment: a target workload, the
//! interventions applied to it, its control type, the diagnosis families it was designed to
//! exercise and the cause the intervention is known to produce. Ground truth is therefore the
//! intervention, never Stutter's own diagnosis output.
use std::collections::BTreeSet;

use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

use crate::validation_campaign::manifest::CampaignManifest;

pub const SCENARIO_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Scheduler,
    Io,
    Irq,
    Gpu,
}

impl Family {
    pub const ALL: [Self; 4] = [Self::Scheduler, Self::Io, Self::Irq, Self::Gpu];

    /// The `xtask calibrate --family` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduler => "scheduler",
            Self::Io => "io",
            Self::Irq => "irq",
            Self::Gpu => "gpu",
        }
    }

    /// The diagnosis this family's intervention is known to produce.
    pub fn positive_cause(self) -> &'static str {
        match self {
            Self::Scheduler => "GameThreadSchedulerDelay",
            Self::Io => "BlockIoCandidate",
            Self::Irq => "IrqDelayCandidate",
            Self::Gpu => "GpuBoundCandidate",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    Positive,
    NegativeControl,
    MixedConfounded,
}

impl ControlType {
    /// The `[provenance].control_type` value the calibrator reads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::NegativeControl => "negative_control",
            Self::MixedConfounded => "mixed_confounded",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Light, latency-sensitive `vkmark` scene (frame source: MangoHud).
    VkmarkLight,
    /// `vkmark` at a frozen GPU-saturating configuration.
    VkmarkGpuHeavy,
    /// `fio` fixed-work process tree.
    FioFixedWork,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    /// SCHED_FIFO contender on the target's CPU (typed runner action).
    SchedulerBurst,
    /// Bounded blocking storage I/O.
    BlockingIo,
    /// Bounded traffic through the local default-gateway NIC to drive its IRQ.
    NetworkTraffic,
    /// Bounded additional GPU stressor.
    GpuStressor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSpec {
    pub id: String,
    pub control: ControlType,
    /// Diagnosis families this capture was designed to exercise (`[coverage].diagnoses`).
    pub covers: Vec<Family>,
    pub target: Target,
    #[serde(default)]
    pub interventions: Vec<InterventionKind>,
    /// Cause the intervention is known to produce; `Unknown` for a negative control.
    pub expected_primary_cause: String,
    pub seconds: u64,
    /// Independent captures required.
    pub count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VkmarkParams {
    pub light_scene: String,
    pub light_size: String,
    pub heavy_scene: String,
    pub heavy_size: String,
    pub present_mode: String,
    pub heavy_present_mode: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSet {
    pub schema_version: u32,
    pub vkmark: VkmarkParams,
    #[serde(rename = "scenario")]
    pub scenarios: Vec<ScenarioSpec>,
}

fn valid_dimension(value: &str) -> bool {
    value.split_once('x').is_some_and(|(w, h)| {
        w.parse::<u32>().is_ok_and(|w| w > 0) && h.parse::<u32>().is_ok_and(|h| h > 0)
    })
}

fn safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl ScenarioSet {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let set: Self = toml::from_str(text)?;
        ensure!(
            set.schema_version == SCENARIO_SCHEMA_VERSION,
            "unsupported scenario schema {}",
            set.schema_version
        );
        set.validate_shape()?;
        Ok(set)
    }

    pub fn get(&self, id: &str) -> Option<&ScenarioSpec> {
        self.scenarios.iter().find(|spec| spec.id == id)
    }

    fn validate_shape(&self) -> anyhow::Result<()> {
        ensure!(
            valid_dimension(&self.vkmark.light_size)
                && valid_dimension(&self.vkmark.heavy_size)
                && safe_token(&self.vkmark.light_scene)
                && safe_token(&self.vkmark.heavy_scene)
                && safe_token(&self.vkmark.present_mode)
                && safe_token(&self.vkmark.heavy_present_mode),
            "vkmark parameters must be a scene token, a WxH size and a present mode"
        );
        let mut ids = BTreeSet::new();
        for spec in &self.scenarios {
            ensure!(
                safe_token(&spec.id),
                "scenario id {:?} is not a safe token",
                spec.id
            );
            ensure!(
                ids.insert(spec.id.as_str()),
                "duplicate scenario {}",
                spec.id
            );
            ensure!(!spec.covers.is_empty(), "{}: covers no family", spec.id);
            ensure!(
                (20..=300).contains(&spec.seconds) && spec.count > 0,
                "{}: seconds must be within 20..=300 and count positive",
                spec.id
            );
            let distinct = spec.interventions.iter().collect::<BTreeSet<_>>();
            ensure!(
                distinct.len() == spec.interventions.len(),
                "{}: repeated intervention",
                spec.id
            );
            match spec.control {
                ControlType::Positive => {
                    let [family] = spec.covers.as_slice() else {
                        bail!("{}: a positive covers exactly one family", spec.id);
                    };
                    ensure!(
                        spec.expected_primary_cause == family.positive_cause(),
                        "{}: a {} positive must expect {}",
                        spec.id,
                        family.as_str(),
                        family.positive_cause()
                    );
                    ensure!(
                        !spec.interventions.is_empty(),
                        "{}: a positive needs an intervention",
                        spec.id
                    );
                }
                ControlType::NegativeControl => {
                    ensure!(
                        spec.expected_primary_cause == "Unknown",
                        "{}: a negative control expects Unknown",
                        spec.id
                    );
                    ensure!(
                        spec.interventions.is_empty(),
                        "{}: a negative control applies no intervention",
                        spec.id
                    );
                }
                ControlType::MixedConfounded => {
                    ensure!(
                        spec.covers.len() >= 2 && spec.interventions.len() >= 2,
                        "{}: a mixed capture combines at least two families and interventions",
                        spec.id
                    );
                    ensure!(
                        spec.covers
                            .iter()
                            .any(|f| f.positive_cause() == spec.expected_primary_cause),
                        "{}: the declared primary cause must belong to a covered family",
                        spec.id
                    );
                }
            }
        }
        Ok(())
    }

    /// Per-family capture counts this set can deliver, by control type.
    pub fn capacity(&self, family: Family) -> (usize, usize, usize) {
        let sum = |control| {
            self.scenarios
                .iter()
                .filter(|s| s.control == control && s.covers.contains(&family))
                .map(|s| s.count)
                .sum::<usize>()
        };
        (
            sum(ControlType::Positive),
            sum(ControlType::NegativeControl),
            sum(ControlType::MixedConfounded),
        )
    }

    /// The set must be able to meet every family's evidence floor, or the manifest and the
    /// scenarios disagree and the campaign could never finish.
    pub fn validate_against(&self, manifest: &CampaignManifest) -> anyhow::Result<()> {
        for family in Family::ALL {
            let (positive, negative, mixed) = self.capacity(family);
            ensure!(
                positive >= manifest.corpus.min_positive
                    && negative >= manifest.corpus.min_negative_control
                    && mixed >= manifest.corpus.min_mixed,
                "scenarios deliver {positive}/{negative}/{mixed} (positive/negative/mixed) for {}, below the manifest floors {}/{}/{}",
                family.as_str(),
                manifest.corpus.min_positive,
                manifest.corpus.min_negative_control,
                manifest.corpus.min_mixed
            );
        }
        Ok(())
    }

    pub fn total_captures(&self) -> usize {
        self.scenarios.iter().map(|s| s.count).sum()
    }
}
