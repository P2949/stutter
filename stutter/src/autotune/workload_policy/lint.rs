use super::{
    r#match::family_matches,
    model::{
        LintSeverity, WorkloadPolicyLint, WorkloadPolicyLintKind, WorkloadPolicyMatrix,
        WorkloadPolicyRule,
    },
};
use crate::{
    autotune::objective::ObjectiveKind,
    daemon::policy::{DaemonMode, DaemonPolicy},
};

pub fn lint_workload_policy(
    matrix: &WorkloadPolicyMatrix,
    policy: &DaemonPolicy,
) -> Vec<WorkloadPolicyLint> {
    let mut lints = Vec::new();

    for rule in &matrix.rules {
        lint_rule(rule, policy, &mut lints);
    }

    lints.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| format!("{:?}", left.situation).cmp(&format!("{:?}", right.situation)))
            .then_with(|| left.family.cmp(&right.family))
    });
    lints
}

pub fn validate_workload_policy_lints(lints: &[WorkloadPolicyLint]) -> anyhow::Result<()> {
    let errors = lints
        .iter()
        .filter(|lint| lint.severity == LintSeverity::Error)
        .map(|lint| lint.message.as_str())
        .collect::<Vec<_>>();

    if errors.is_empty() {
        return Ok(());
    }

    anyhow::bail!("critical workload policy lint(s): {}", errors.join("; "))
}

fn lint_rule(
    rule: &WorkloadPolicyRule,
    policy: &DaemonPolicy,
    lints: &mut Vec<WorkloadPolicyLint>,
) {
    if rule.autonomous_families.is_empty() && !rule.allowed_families.is_empty() {
        lints.push(WorkloadPolicyLint::warning(
            WorkloadPolicyLintKind::EmptyAutonomousFamilies,
            format!(
                "{:?} allows suggestions but has no autonomous families; live apply will not choose these candidates",
                rule.situation
            ),
            Some(rule.situation),
            None,
        ));
    }

    for family in &rule.autonomous_families {
        if policy
            .denied_action_families
            .iter()
            .any(|denied| family_matches(family, denied) || family_matches(denied, family))
        {
            lints.push(WorkloadPolicyLint::error(
                WorkloadPolicyLintKind::DeniedFamilyIsAutonomous,
                format!(
                    "{:?} makes denied action family {family:?} autonomous",
                    rule.situation
                ),
                Some(rule.situation),
                Some(family),
            ));
        }

        let family_enabled_for_apply = policy.mode.supports_apply();
        if family_enabled_for_apply && high_risk_family(family) && !policy.allow_high_risk {
            lints.push(WorkloadPolicyLint::error(
                WorkloadPolicyLintKind::HighRiskFamilyIsAutonomous,
                format!(
                    "{:?} makes high-risk action family {family:?} autonomous while high-risk apply is disabled",
                    rule.situation
                ),
                Some(rule.situation),
                Some(family),
            ));
        }

        if family_enabled_for_apply
            && system_wide_family(family)
            && !policy.allow_system_wide_apply
            && !medium_risk_system_family_allowed(family, policy)
        {
            lints.push(WorkloadPolicyLint::error(
                WorkloadPolicyLintKind::MediumRiskSystemWideDenied,
                format!(
                    "{:?} makes system-wide action family {family:?} autonomous while system-wide apply is disabled",
                    rule.situation
                ),
                Some(rule.situation),
                Some(family),
            ));
        }

        if family_enabled_for_apply
            && policy.mode == DaemonMode::ApplyLowRisk
            && medium_or_high_risk_family(family)
        {
            lints.push(WorkloadPolicyLint::error(
                WorkloadPolicyLintKind::ApplyLowRiskAutonomousFamilyTooRisky,
                format!(
                    "{:?} makes medium/high-risk action family {family:?} autonomous in apply-low-risk mode",
                    rule.situation
                ),
                Some(rule.situation),
                Some(family),
            ));
        }
    }

    for objective in &rule.allowed_objectives {
        if !rule
            .allowed_families
            .iter()
            .any(|family| family_supports_objective(family, *objective))
        {
            lints.push(WorkloadPolicyLint::warning(
                WorkloadPolicyLintKind::ObjectiveWithoutCapableFamily,
                format!(
                    "{:?} allows objective {:?} but no allowed action family is expected to optimize it",
                    rule.situation, objective
                ),
                Some(rule.situation),
                None,
            ));
        }
    }
}

fn high_risk_family(family: &str) -> bool {
    matches!(family, "high_risk")
}

fn system_wide_family(family: &str) -> bool {
    matches!(
        family,
        "cpu_power" | "gpu_power" | "irq_affinity" | "vm_knob"
    )
}

fn medium_risk_system_family_allowed(family: &str, policy: &DaemonPolicy) -> bool {
    if policy.mode != DaemonMode::ApplyMediumRisk || !policy.allow_medium_risk_apply {
        return false;
    }

    match family {
        "irq_affinity" => true,
        "cpu_power" | "gpu_power" => policy.allow_gpu_power_in_autotune,
        "vm_knob" => policy.allow_vm_knobs_in_autotune,
        _ => false,
    }
}

fn medium_or_high_risk_family(family: &str) -> bool {
    !matches!(family, "cpu_affinity_profile")
}

fn family_supports_objective(family: &str, objective: ObjectiveKind) -> bool {
    use ObjectiveKind::*;

    match family {
        "cpu_affinity_profile" => matches!(
            objective,
            StutterScore
                | GameFramePacing
                | GameRunnableLatency
                | DesktopInteractivity
                | CompileThroughputWithForegroundProtection
        ),
        "nice" | "uclamp" => matches!(
            objective,
            StutterScore
                | DesktopInteractivity
                | BrowserInteractivity
                | CompileThroughputWithForegroundProtection
                | GameRunnableLatency
        ),
        "ionice" => matches!(objective, StutterScore | DesktopInteractivity | IoLatency),
        "cgroup_placement" => matches!(
            objective,
            StutterScore
                | DesktopInteractivity
                | CompileThroughputWithForegroundProtection
                | GameRunnableLatency
        ),
        "irq_affinity" => matches!(objective, StutterScore | IrqOverlapReduction),
        "cpu_power" | "gpu_power" => matches!(
            objective,
            StutterScore | ThermalRecovery | GameFramePacing | BrowserInteractivity
        ),
        "vm_knob" => matches!(objective, StutterScore | IoLatency),
        _ => false,
    }
}
