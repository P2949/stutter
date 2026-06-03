use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionId, SafetyClass},
    daemon_policy::{
        ActionDescriptor, ActionEffectScope, ActionSource, DaemonPolicy, PolicyIntent,
        RollbackRequirement,
    },
    diagnosis::{Confidence, StutterCause},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorFixKind {
    CpuAffinityProfile,
    NicePriority,
    IoPriority,
    UClamp,
    CgroupPlacement,
    IrqAffinityInvestigation,
    GpuPowerInvestigation,
    DisplayPathInvestigation,
    BlockIoInvestigation,
    CollectMoreData,
}

impl AdvisorFixKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::CpuAffinityProfile => "cpu_affinity_profile",
            Self::NicePriority => "nice_priority",
            Self::IoPriority => "io_priority",
            Self::UClamp => "uclamp",
            Self::CgroupPlacement => "cgroup_placement",
            Self::IrqAffinityInvestigation => "irq_affinity_investigation",
            Self::GpuPowerInvestigation => "gpu_power_investigation",
            Self::DisplayPathInvestigation => "display_path_investigation",
            Self::BlockIoInvestigation => "block_io_investigation",
            Self::CollectMoreData => "collect_more_data",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorFixValidationStatus {
    NotRun,
    Validated,
    Rejected,
    Inconclusive,
    Underpowered,
    UnsafeToRun,
    InvalidExperiment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorExpectedMetricMovement {
    pub metric: String,
    pub lower_is_better: bool,
    pub minimum_relative_improvement_percent: Option<f64>,
    pub maximum_allowed_regression_percent: Option<f64>,
    pub required_ci_excludes_zero: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorValidationRecipe {
    pub baseline_runs_required: usize,
    pub test_runs_required: usize,
    pub scenario_name: Option<String>,
    pub baseline_command: String,
    pub experiment_command: String,
    pub compare_command: String,
    pub stop_conditions: Vec<String>,
    pub acceptance_criteria: Vec<AdvisorExpectedMetricMovement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorSafetyRisk {
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback_requirement: RollbackRequirement,
    pub requires_privilege: bool,
    pub system_wide: bool,
    pub persistent: bool,
    pub allowed_by_default_policy: bool,
    pub required_policy_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorFixPlan {
    pub schema_version: u32,
    pub id: String,
    pub kind: AdvisorFixKind,
    pub cause: StutterCause,
    pub confidence: Confidence,
    pub rationale: String,
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback: RollbackRequirement,
    pub safety_risk: AdvisorSafetyRisk,
    pub expected_metric_movement: Vec<AdvisorExpectedMetricMovement>,
    pub validation: AdvisorValidationRecipe,
    pub suggested_commands: Vec<String>,
    pub candidate_plan_path: Option<PathBuf>,
    pub safety_notes: Vec<String>,
}

pub(crate) fn scheduler_profile_fix_plan(
    run: &Path,
    cause: StutterCause,
    tree_pid: Option<u32>,
    profiles: Option<&Path>,
    evidence: Option<String>,
) -> AdvisorFixPlan {
    let pid_arg = tree_pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "<PID>".to_owned());
    let profiles_arg = profiles
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<profiles.toml>".to_owned());
    let experiment_command = format!(
        "stutter tune --tree-pid {pid_arg} --profiles {profiles_arg} --runs 5 --baseline-profile baseline-online"
    );
    let expected_metric_movement = scheduler_expected_metric_movement();
    let safety_class = SafetyClass::ReversibleLowRisk;
    let effect_scope = ActionEffectScope::LocalProcessTree;
    let rollback = RollbackRequirement::RequiredBeforeApply;

    AdvisorFixPlan {
        schema_version: 1,
        id: format!("advisor-fix:{}:cpu-affinity-profile", cause_id(cause)),
        kind: AdvisorFixKind::CpuAffinityProfile,
        cause,
        confidence: Confidence::Medium,
        rationale: evidence.unwrap_or_else(|| {
            "Scheduler-delay evidence suggests isolating latency-sensitive game or compositor threads may reduce delayed wakeups.".to_owned()
        }),
        safety_class: safety_class.clone(),
        effect_scope,
        rollback,
        safety_risk: safety_risk_for(
            "cpu_affinity_profile",
            safety_class,
            effect_scope,
            rollback,
            true,
            false,
        ),
        expected_metric_movement: expected_metric_movement.clone(),
        validation: AdvisorValidationRecipe {
            baseline_runs_required: 5,
            test_runs_required: 5,
            scenario_name: None,
            baseline_command: format!(
                "stutter record --tree-pid {pid_arg} --duration 180 --run-name baseline-a"
            ),
            experiment_command: experiment_command.clone(),
            compare_command: format!(
                "stutter recommend --fix-plan {} --baseline <baseline-run> --tune <tune-dir> --html fix-validation.html",
                run.join("advisor_fix_plan_cpu_affinity_profile.json").display()
            ),
            stop_conditions: vec![
                "Stop if the target process tree changes substantially between baseline and test runs.".to_owned(),
                "Stop if frame data, scheduler data, or diagnostic scores are missing from either side.".to_owned(),
                "Stop if any safety metric regresses beyond the acceptance guardrail.".to_owned(),
            ],
            acceptance_criteria: expected_metric_movement,
        },
        suggested_commands: vec![experiment_command],
        candidate_plan_path: Some(run.join("advisor_fix_plan_cpu_affinity_profile.json")),
        safety_notes: vec![
            "Applyable experiment only after repeated baseline/test evidence; do not treat one run as proof.".to_owned(),
            "Rollback must be available before applying any profile result.".to_owned(),
        ],
    }
}

pub(crate) fn gpu_investigation_fix_plan(
    evidence: Option<String>,
    has_hwmon: bool,
) -> AdvisorFixPlan {
    let kind = if has_hwmon {
        AdvisorFixKind::DisplayPathInvestigation
    } else {
        AdvisorFixKind::GpuPowerInvestigation
    };
    investigation_fix_plan(
        "advisor-fix:gpu-bound:investigation",
        kind,
        StutterCause::GpuBoundCandidate,
        evidence.unwrap_or_else(|| {
            "GPU-bound evidence is a candidate, not proof; collect GPU power, DRM fence, and display-path data before trying CPU affinity.".to_owned()
        }),
        if has_hwmon {
            vec![
                "stutter report --analysis-json <run-dir>".to_owned(),
                "stutter display-path compare --baseline <baseline-run> --test <test-run>".to_owned(),
            ]
        } else {
            vec![
                "stutter record --hwmon --drm-fence-latency --duration 180 --run-name gpu-check"
                    .to_owned(),
            ]
        },
        vec![
            "Collect hwmon GPU power/clock evidence when available.".to_owned(),
            "Compare DRM fence and display-path timing before applying CPU tuning.".to_owned(),
        ],
    )
}

pub(crate) fn irq_investigation_fix_plan(
    evidence: Option<String>,
    has_irq: bool,
) -> AdvisorFixPlan {
    investigation_fix_plan(
        "advisor-fix:irq-delay:investigation",
        AdvisorFixKind::IrqAffinityInvestigation,
        StutterCause::IrqDelayCandidate,
        evidence.unwrap_or_else(|| {
            "IRQ overlap is a candidate signal; current policy treats IRQ affinity as manual investigation only.".to_owned()
        }),
        if has_irq {
            vec!["stutter report --analysis-json <run-dir>".to_owned()]
        } else {
            vec![
                "stutter record --irq-latency --irq <IRQ> --duration 180 --run-name irq-check"
                    .to_owned(),
            ]
        },
        vec![
            "Do not apply IRQ affinity automatically; the daemon contract forbids this by default.".to_owned(),
            "Inspect the specific interrupt and target-thread CPU overlap before proposing any manual change.".to_owned(),
        ],
    )
}

pub(crate) fn block_io_investigation_fix_plan(
    evidence: Option<String>,
    has_block_io: bool,
) -> AdvisorFixPlan {
    investigation_fix_plan(
        "advisor-fix:block-io:investigation",
        AdvisorFixKind::BlockIoInvestigation,
        StutterCause::BlockIoCandidate,
        evidence.unwrap_or_else(|| {
            "Block I/O overlap is a candidate; confirm storage pressure before CPU tuning."
                .to_owned()
        }),
        if has_block_io {
            vec!["stutter report --analysis-json <run-dir>".to_owned()]
        } else {
            vec!["stutter record --block-io --duration 180 --run-name io-check".to_owned()]
        },
        vec![
            "Observe only; CPU affinity is not the first fix for a storage-pressure candidate."
                .to_owned(),
        ],
    )
}

pub(crate) fn collect_more_data_fix_plan(rationale: String, command: String) -> AdvisorFixPlan {
    investigation_fix_plan(
        "advisor-fix:collect-more-data",
        AdvisorFixKind::CollectMoreData,
        StutterCause::Unknown,
        rationale,
        vec![command],
        vec!["Observe only; do not apply tuning from low-quality or inconclusive data.".to_owned()],
    )
}

fn scheduler_expected_metric_movement() -> Vec<AdvisorExpectedMetricMovement> {
    vec![
        AdvisorExpectedMetricMovement {
            metric: "diagnostic_raw_score_total".to_owned(),
            lower_is_better: true,
            minimum_relative_improvement_percent: Some(5.0),
            maximum_allowed_regression_percent: None,
            required_ci_excludes_zero: true,
        },
        AdvisorExpectedMetricMovement {
            metric: "over_5ms".to_owned(),
            lower_is_better: true,
            minimum_relative_improvement_percent: Some(10.0),
            maximum_allowed_regression_percent: None,
            required_ci_excludes_zero: true,
        },
        AdvisorExpectedMetricMovement {
            metric: "frame_p99_ms".to_owned(),
            lower_is_better: true,
            minimum_relative_improvement_percent: None,
            maximum_allowed_regression_percent: Some(5.0),
            required_ci_excludes_zero: false,
        },
    ]
}

fn investigation_fix_plan(
    id: &str,
    kind: AdvisorFixKind,
    cause: StutterCause,
    rationale: String,
    suggested_commands: Vec<String>,
    safety_notes: Vec<String>,
) -> AdvisorFixPlan {
    let safety_class = SafetyClass::ObserveOnly;
    let effect_scope = match kind {
        AdvisorFixKind::IrqAffinityInvestigation => ActionEffectScope::Irq,
        _ => ActionEffectScope::ObserveOnly,
    };
    let rollback = match kind {
        AdvisorFixKind::IrqAffinityInvestigation => RollbackRequirement::Unavailable,
        _ => RollbackRequirement::NotRequiredForDryRun,
    };
    let acceptance_criteria = vec![AdvisorExpectedMetricMovement {
        metric: "diagnostic_raw_score_total".to_owned(),
        lower_is_better: true,
        minimum_relative_improvement_percent: None,
        maximum_allowed_regression_percent: None,
        required_ci_excludes_zero: false,
    }];

    AdvisorFixPlan {
        schema_version: 1,
        id: id.to_owned(),
        kind,
        cause,
        confidence: Confidence::Medium,
        rationale,
        safety_class: safety_class.clone(),
        effect_scope,
        rollback,
        safety_risk: safety_risk_for(
            "observe_only_investigation",
            safety_class,
            effect_scope,
            rollback,
            false,
            false,
        ),
        expected_metric_movement: Vec::new(),
        validation: AdvisorValidationRecipe {
            baseline_runs_required: 1,
            test_runs_required: 1,
            scenario_name: None,
            baseline_command: "stutter record --duration 180 --run-name investigation-baseline"
                .to_owned(),
            experiment_command: suggested_commands
                .first()
                .cloned()
                .unwrap_or_else(|| "stutter report --analysis-json <run-dir>".to_owned()),
            compare_command: "stutter report --analysis-json <run-dir>".to_owned(),
            stop_conditions: vec![
                "Stop if required evidence streams are unavailable.".to_owned(),
                "Do not apply system-wide or IRQ/GPU/block-I/O changes from this plan.".to_owned(),
            ],
            acceptance_criteria,
        },
        suggested_commands,
        candidate_plan_path: None,
        safety_notes,
    }
}

fn safety_risk_for(
    action_kind: &str,
    safety_class: SafetyClass,
    effect_scope: ActionEffectScope,
    rollback: RollbackRequirement,
    requires_explicit_target: bool,
    persistent: bool,
) -> AdvisorSafetyRisk {
    let system_wide = matches!(
        effect_scope,
        ActionEffectScope::Irq
            | ActionEffectScope::Sysfs
            | ActionEffectScope::CpuPower
            | ActionEffectScope::GpuPower
            | ActionEffectScope::VmKnob
            | ActionEffectScope::SystemWide
    );
    let descriptor = ActionDescriptor {
        action_id: ActionId::new(format!("advisor:{action_kind}")),
        action_kind: action_kind.to_owned(),
        safety_class: safety_class.clone(),
        effect_scope,
        rollback,
        persistent_effect: persistent,
        touches_system_wide_state: system_wide,
        requires_explicit_target,
        confidence: None,
    };
    let (policy, intent, required_policy_mode) = if safety_class == SafetyClass::ObserveOnly {
        (
            DaemonPolicy::observe(ActionSource::Cli),
            PolicyIntent::Observe,
            "observe",
        )
    } else {
        (
            DaemonPolicy::apply_low_risk(ActionSource::Cli),
            PolicyIntent::Apply,
            required_policy_mode_for(&safety_class),
        )
    };

    AdvisorSafetyRisk {
        safety_class,
        effect_scope,
        rollback_requirement: rollback,
        requires_privilege: effect_scope != ActionEffectScope::ObserveOnly,
        system_wide,
        persistent,
        allowed_by_default_policy: policy.check_action(intent, &descriptor).is_ok(),
        required_policy_mode: required_policy_mode.to_owned(),
    }
}

fn required_policy_mode_for(safety_class: &SafetyClass) -> &'static str {
    match safety_class {
        SafetyClass::ObserveOnly => "observe",
        SafetyClass::ReversibleLowRisk => "apply-low-risk",
        SafetyClass::ReversibleMediumRisk => "apply-medium-risk",
        SafetyClass::HighRisk => "apply-high-risk",
    }
}

fn cause_id(cause: StutterCause) -> &'static str {
    match cause {
        StutterCause::GameThreadSchedulerDelay => "game-thread-scheduler-delay",
        StutterCause::CompositorSchedulerDelay => "compositor-scheduler-delay",
        StutterCause::GpuBoundCandidate => "gpu-bound",
        StutterCause::IrqDelayCandidate => "irq-delay",
        StutterCause::BlockIoCandidate => "block-io",
        StutterCause::CpuPressureCandidate => "cpu-pressure",
        StutterCause::CpuMonopolizationCandidate => "cpu-monopolization",
        StutterCause::RuntimeWaitCandidate => "runtime-wait",
        StutterCause::Unknown => "unknown",
    }
}
