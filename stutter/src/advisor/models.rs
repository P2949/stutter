use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    advisor::fix_plan::{AdvisorFixPlan, AdvisorSafetyRisk},
    diagnosis::{Confidence, StutterCause},
    irq_inspect::{IrqDeviceClass, IrqLine},
    process_tree::TaskClass,
    report::DataQualityLevel,
};

#[derive(Debug, Clone)]
pub struct AdvisorCommandInput {
    pub run: Option<PathBuf>,
    pub profiles: Option<PathBuf>,
    pub json: bool,
    pub watch_runs: bool,
    pub runs_dir: Option<PathBuf>,
    pub poll_seconds: u64,
    pub once: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorReport {
    pub schema_version: u32,
    pub run: PathBuf,
    pub data_quality: DataQualityLevel,
    pub verdict: AdvisorVerdict,
    pub recommendations: Vec<AdvisorRecommendation>,
    #[serde(default)]
    pub fix_plans: Vec<AdvisorFixPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdvisorVerdict {
    NoAction,
    CollectMoreData,
    TryProfileTuning,
    InvestigateNonCpuBottleneck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorRecommendation {
    pub title: String,
    pub rationale: String,
    pub confidence: Confidence,
    pub suggested_commands: Vec<String>,
    pub safety_note: String,
    pub safety_risk: AdvisorSafetyRisk,
    #[serde(default)]
    pub fix_plan: Option<AdvisorFixPlan>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdvisorCauseEvidence {
    pub cause: StutterCause,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdvisorTargetAffinityOverlap {
    pub task: u32,
    pub comm: String,
    pub class: TaskClass,
    pub allowed_cpus: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdvisorIrqAffinityOverlap {
    pub irq: u32,
    pub irq_cpu: u32,
    pub irq_name: String,
    pub irq_class: IrqDeviceClass,
    pub overlapping_tasks: Vec<AdvisorTargetAffinityOverlap>,
}

pub(crate) struct AdvisorSignalAvailability {
    pub has_hwmon: bool,
    pub has_irq: bool,
    pub has_block_io: bool,
}

pub(crate) struct AdvisorEvidenceInput<'a> {
    pub run: &'a Path,
    pub data_quality: DataQualityLevel,
    pub causes: &'a [StutterCause],
    pub cause_evidence: &'a [AdvisorCauseEvidence],
    pub profiles: Option<&'a Path>,
    pub signal_availability: AdvisorSignalAvailability,
    pub tree_pid: Option<u32>,
    pub irq_inventory: &'a [IrqLine],
    pub irq_affinity_overlaps: &'a [AdvisorIrqAffinityOverlap],
}
