//! The post-v0.1 diagnosis study definition (`docs/validation/scenarios/diagnosis-v2.toml`).
//! Each family is a *matched pair*: the same target and configuration with and without the
//! predeclared intervention. Ground truth is the intervention, never a diagnosis result.
use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

use crate::validation_campaign::scenarios::model::{
    ControlType, Family, InterventionKind, Placement, ScenarioSet, ScenarioSpec, Target,
    VkmarkParams,
};

pub const STUDY_SCHEMA_VERSION: u32 = 1;
pub const CONFIG_FILE: &str = "docs/validation/scenarios/diagnosis-v2.toml";

/// The direct, user-facing effect measured for a family. Never a diagnosis output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    RunnableP99Ns,
    FrameP99Ms,
    TargetElapsedMs,
}

impl Metric {
    pub fn name(self) -> &'static str {
        match self {
            Self::RunnableP99Ns => "target_runnable_p99",
            Self::FrameP99Ms => "target_frame_p99",
            Self::TargetElapsedMs => "fixed_work_elapsed",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::RunnableP99Ns => "ns",
            Self::FrameP99Ms => "ms",
            Self::TargetElapsedMs => "ms",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sampling {
    pub initial_pairs: usize,
    pub max_pairs: usize,
    pub extension_step: usize,
    /// |Cohen's d-like effect size| the intervention must show in the worsening direction.
    pub min_effect_size: f64,
    pub failed_attempts_per_capture: usize,
    pub seconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyDef {
    pub family: Family,
    pub target: Target,
    pub intervention: InterventionKind,
    #[serde(default)]
    pub placement: Placement,
    pub metric: Metric,
    #[serde(default)]
    pub accepted_wakeup_coalescing: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MixedDef {
    pub id: String,
    pub covers: Vec<Family>,
    pub target: Target,
    pub interventions: Vec<InterventionKind>,
    #[serde(default)]
    pub placement: Placement,
    pub expected_primary_cause: String,
    pub count: usize,
    #[serde(default)]
    pub accepted_wakeup_coalescing: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StudyConfig {
    pub study_schema_version: u32,
    pub study: String,
    pub vkmark: VkmarkParams,
    pub sampling: Sampling,
    #[serde(rename = "family")]
    pub families: Vec<FamilyDef>,
    #[serde(rename = "mixed", default)]
    pub mixed: Vec<MixedDef>,
}

impl StudyConfig {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text)?;
        ensure!(
            config.study_schema_version == STUDY_SCHEMA_VERSION,
            "unsupported study schema {}",
            config.study_schema_version
        );
        config.validate()?;
        Ok(config)
    }

    pub fn family(&self, family: Family) -> Option<&FamilyDef> {
        self.families.iter().find(|def| def.family == family)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let s = &self.sampling;
        ensure!(
            (15..=30).contains(&s.initial_pairs)
                && s.initial_pairs <= s.max_pairs
                && s.max_pairs <= 30
                && s.extension_step > 0,
            "sampling needs 15 <= initial_pairs <= max_pairs <= 30 and a positive step"
        );
        ensure!(
            s.min_effect_size > 0.0 && s.failed_attempts_per_capture > 0,
            "effect size and failure budget must be positive"
        );
        for family in Family::ALL {
            let count = self.families.iter().filter(|d| d.family == family).count();
            ensure!(
                count == 1,
                "family {} must be defined once",
                family.as_str()
            );
        }
        for def in &self.families {
            ensure!(
                def.intervention_matches(),
                "{}: the intervention does not belong to the family",
                def.family.as_str()
            );
        }
        for mixed in &self.mixed {
            ensure!(
                mixed.count >= 6,
                "{}: mixed captures need count >= 6",
                mixed.id
            );
        }
        // Reuse the production scenario rules for shapes, tokens and cause/family agreement.
        let set = ScenarioSet {
            schema_version: 1,
            vkmark: self.vkmark.clone(),
            scenarios: self.scenarios(),
        };
        set.validate_shape()
    }

    /// Every scenario the study can run, in a stable order.
    pub fn scenarios(&self) -> Vec<ScenarioSpec> {
        let mut specs = Vec::new();
        for def in &self.families {
            specs.push(self.control_spec(def));
            specs.push(self.positive_spec(def));
        }
        specs.extend(self.mixed.iter().map(|m| self.mixed_spec(m)));
        specs
    }

    fn base(&self, id: String, control: ControlType) -> ScenarioSpec {
        ScenarioSpec {
            id,
            control,
            covers: Vec::new(),
            target: Target::VkmarkLight,
            interventions: Vec::new(),
            expected_primary_cause: "Unknown".to_owned(),
            seconds: self.sampling.seconds,
            count: 1,
            accepted_wakeup_coalescing: 0,
            placement: Placement::Unpinned,
            informational: true,
        }
    }

    pub fn control_spec(&self, def: &FamilyDef) -> ScenarioSpec {
        ScenarioSpec {
            covers: vec![def.family],
            target: def.target,
            placement: def.placement,
            accepted_wakeup_coalescing: def.accepted_wakeup_coalescing,
            ..self.base(
                format!("v2-{}-control", def.family.as_str()),
                ControlType::NegativeControl,
            )
        }
    }

    pub fn positive_spec(&self, def: &FamilyDef) -> ScenarioSpec {
        ScenarioSpec {
            covers: vec![def.family],
            target: def.target,
            placement: def.placement,
            interventions: vec![def.intervention],
            expected_primary_cause: def.family.positive_cause().to_owned(),
            accepted_wakeup_coalescing: def.accepted_wakeup_coalescing,
            ..self.base(
                format!("v2-{}-positive", def.family.as_str()),
                ControlType::Positive,
            )
        }
    }

    pub fn mixed_spec(&self, def: &MixedDef) -> ScenarioSpec {
        ScenarioSpec {
            covers: def.covers.clone(),
            target: def.target,
            placement: def.placement,
            interventions: def.interventions.clone(),
            expected_primary_cause: def.expected_primary_cause.clone(),
            accepted_wakeup_coalescing: def.accepted_wakeup_coalescing,
            count: def.count,
            ..self.base(def.id.clone(), ControlType::MixedConfounded)
        }
    }

    pub fn scenario(&self, id: &str) -> anyhow::Result<ScenarioSpec> {
        match self.scenarios().into_iter().find(|s| s.id == id) {
            Some(spec) => Ok(spec),
            None => bail!("unknown study scenario {id}"),
        }
    }
}

impl FamilyDef {
    fn intervention_matches(&self) -> bool {
        matches!(
            (self.family, self.intervention),
            (Family::Scheduler, InterventionKind::SchedulerBurst)
                | (Family::Io, InterventionKind::BlockingIo)
                | (Family::Irq, InterventionKind::NetworkTraffic)
                | (Family::Gpu, InterventionKind::GpuStressor)
        )
    }
}
