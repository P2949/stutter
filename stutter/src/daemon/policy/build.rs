//! Daemon policy construction from daemon configuration; this module must not own evaluation rules.

use std::collections::BTreeSet;

use super::{
    ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy, HIGH_RISK_APPLY_IMPLEMENTED,
    context::DaemonPolicyBuildInput, remote::remote_apply_policy_for_config,
};
use crate::{
    actions::SafetyClass,
    daemon::config::{DaemonCandidateConfidenceConfig, DaemonConfig},
};

pub fn build_daemon_policy(input: DaemonPolicyBuildInput<'_>) -> DaemonPolicy {
    let config = input.config;
    let remote_context = input.remote_context.as_ref();
    let mode_max_safety_class = max_safety_class_for_mode(config.mode);
    let mut max_safety_class = mode_max_safety_class.clone();

    if let Some(context) = remote_context
        && context.limits.max_safety_class < max_safety_class
    {
        max_safety_class = context.limits.max_safety_class.clone();
    }

    let min_confidence = min_confidence_for_config(config);
    let confidence = confidence_thresholds_for_config(config, min_confidence);

    let mut allow_high_risk = HIGH_RISK_APPLY_IMPLEMENTED
        && config.mode == DaemonMode::ApplyHighRisk
        && config.safety.allow_high_risk;
    let allow_medium_risk_apply =
        config.mode == DaemonMode::ApplyMediumRisk && config.autotune.allow_medium_risk_apply;
    let mut allow_system_wide_suggestions =
        config.mode != DaemonMode::Observe && config.safety.allow_system_wide_suggestions;
    let mut allow_system_wide_apply = HIGH_RISK_APPLY_IMPLEMENTED
        && config.mode == DaemonMode::ApplyHighRisk
        && config.safety.allow_high_risk
        && config.safety.allow_system_wide_apply;

    if let Some(context) = remote_context {
        allow_system_wide_suggestions &= context.limits.allow_system_wide_suggestions;
        allow_system_wide_apply &= context.limits.allow_system_wide_apply;
        allow_high_risk &= context.limits.allow_high_risk;
    }

    DaemonPolicy {
        mode: config.mode,
        source: config.source,
        max_safety_class,
        allowed_effect_scopes: allowed_effect_scopes_for_mode(config.mode),
        enabled_action_families: config.safety.enabled_action_families.clone(),
        denied_action_families: config.safety.denied_action_families.clone(),
        cgroup_targets: config.safety.cgroup_targets.clone(),
        system_wide_allowlist: config.safety.system_wide_allowlist.clone(),
        rollback_required_before_apply: config.mode.supports_apply(),
        allow_medium_risk_apply,
        allow_system_wide_suggestions,
        allow_system_wide_apply,
        allow_high_risk,
        allow_persistent_effects: config.safety.allow_persistent_effects,
        allow_cpu_power_on_battery: config.autotune.allow_cpu_power_on_battery,
        allow_gpu_power_in_autotune: config.autotune.allow_gpu_power_in_autotune,
        allow_vm_knobs_in_autotune: config.autotune.allow_vm_knobs_in_autotune,
        high_risk_dry_run: config.mode == DaemonMode::Suggest && config.autotune.high_risk_dry_run,
        min_confidence,
        confidence,
        remote_apply: remote_apply_policy_for_config(config, remote_context),
    }
}

pub(in crate::daemon::policy) fn max_safety_class_for_mode(mode: DaemonMode) -> SafetyClass {
    match mode {
        DaemonMode::Observe => SafetyClass::ObserveOnly,
        DaemonMode::Suggest => SafetyClass::HighRisk,
        DaemonMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
        DaemonMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
        DaemonMode::ApplyHighRisk => SafetyClass::HighRisk,
    }
}

fn allowed_effect_scopes_for_mode(mode: DaemonMode) -> BTreeSet<ActionEffectScope> {
    match mode {
        DaemonMode::ApplyLowRisk => BTreeSet::from([
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
        ]),
        DaemonMode::ApplyMediumRisk => BTreeSet::from([
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
            ActionEffectScope::Cgroup,
            ActionEffectScope::Irq,
            ActionEffectScope::CpuPower,
            ActionEffectScope::GpuPower,
            ActionEffectScope::VmKnob,
        ]),
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyHighRisk => BTreeSet::from([
            ActionEffectScope::ObserveOnly,
            ActionEffectScope::LocalProcess,
            ActionEffectScope::LocalProcessTree,
            ActionEffectScope::UserStateFile,
            ActionEffectScope::Cgroup,
            ActionEffectScope::Irq,
            ActionEffectScope::Sysfs,
            ActionEffectScope::CpuPower,
            ActionEffectScope::GpuPower,
            ActionEffectScope::VmKnob,
            ActionEffectScope::SystemWide,
        ]),
    }
}

fn min_confidence_for_config(config: &DaemonConfig) -> f32 {
    if config.safety.min_confidence > 0.0 {
        config.safety.min_confidence
    } else if config.mode.supports_apply() {
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    } else {
        0.0
    }
}

fn confidence_thresholds_for_config(
    config: &DaemonConfig,
    min_confidence: f32,
) -> DaemonCandidateConfidenceConfig {
    let configured = config.autotune.confidence;

    DaemonCandidateConfidenceConfig {
        min_suggest_confidence: configured.min_suggest_confidence,
        min_apply_low_risk_confidence: configured.min_apply_low_risk_confidence.max(min_confidence),
        min_apply_medium_risk_confidence: configured
            .min_apply_medium_risk_confidence
            .max(min_confidence)
            .max(0.85),
        min_high_risk_suggestion_confidence: configured.min_high_risk_suggestion_confidence,
    }
}

pub(super) fn config_for_constructor(
    mode: DaemonMode,
    source: ActionSource,
    allow_high_risk: bool,
) -> DaemonConfig {
    let mut config = DaemonConfig {
        mode,
        source,
        ..DaemonConfig::default()
    };
    config.safety.max_safety_class = max_safety_class_for_mode(mode);
    config.safety.allow_high_risk = allow_high_risk;
    config.autotune.allow_medium_risk_apply = mode == DaemonMode::ApplyMediumRisk;
    config.safety.min_confidence = min_confidence_for_config(&config);
    config
}

impl DaemonPolicy {
    pub fn observe(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::Observe, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn suggest(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::Suggest, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_low_risk(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyLowRisk, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_medium_risk(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyMediumRisk, source, false);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    pub fn apply_high_risk_explicit(source: ActionSource) -> Self {
        let config = config_for_constructor(DaemonMode::ApplyHighRisk, source, true);
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }
}
