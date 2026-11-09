use crate::{
    actions::cpu_power::CpuPowerAction,
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::CpuPowerActionPlan,
        },
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            signal_quality_confidence_weight,
        },
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuPowerCandidateEvidence {
    pub policy: String,
    pub related_cpus: Vec<u32>,
    pub current_governor: Option<String>,
    pub current_epp: Option<String>,
    pub thermal_headroom: bool,
    pub ac_power: Option<bool>,
    pub battery_present: bool,
    pub battery_discharging: Option<bool>,
    pub cpu_power_on_battery_allowed: bool,
}

#[derive(Default)]
pub struct CpuPowerProvider;

impl CandidateProvider for CpuPowerProvider {
    fn family(&self) -> &'static str {
        "cpu_power"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::GameCpuSchedulerPressure | SituationKind::CompileCpuBound
        ) || !input.system_health.ok_for_apply
        {
            return Vec::new();
        }

        let Some(structured_evidence) = cpu_power_evidence(input) else {
            return Vec::new();
        };

        let epp = input
            .system_context
            .inventory
            .cpu_policies
            .iter()
            .find(|policy| policy.policy == structured_evidence.policy)
            .and_then(|policy| policy.energy_performance_available_preferences.as_deref())
            .filter(|available| supports_token(Some(available), "performance"))
            .map(|_| "performance".to_owned());

        let Some(epp) = epp else {
            return Vec::new();
        };

        let action = CpuPowerAction {
            sysfs_root: std::path::PathBuf::from("/sys"),
            cpus: structured_evidence.related_cpus.clone(),
            scaling_governor: None,
            energy_performance_preference: Some(epp),
        };
        let objective = match input.observation.primary_situation {
            SituationKind::CompileCpuBound => {
                ObjectiveKind::CompileThroughputWithForegroundProtection
            }
            _ => ObjectiveKind::GameRunnableLatency,
        };
        let confidence = cpu_power_confidence(input, &structured_evidence);
        let candidate = CandidateAction::CpuPower {
            plan: CpuPowerActionPlan {
                name: format!(
                    "cpu-power-policy-{}-performance",
                    structured_evidence.policy
                ),
                action,
                evidence: vec![CandidateEvidence::new(
                    "cpu_power_structured",
                    format!(
                        "policy={} related_cpus={:?} governor={:?} epp={:?} thermal_headroom={} ac_power={:?} battery_present={} battery_discharging={:?} battery_override={} limited_cpu={:?}",
                        structured_evidence.policy,
                        structured_evidence.related_cpus,
                        structured_evidence.current_governor,
                        structured_evidence.current_epp,
                        structured_evidence.thermal_headroom,
                        structured_evidence.ac_power,
                        structured_evidence.battery_present,
                        structured_evidence.battery_discharging,
                        structured_evidence.cpu_power_on_battery_allowed,
                        input.observation.objective_signals.cpu_power_limited_cpu
                    ),
                    confidence,
                )],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence,
            deny_reasons: Vec::new(),
            rank_hint: 80,
        }]
    }
}

fn cpu_power_evidence(input: &CandidateProviderInput<'_>) -> Option<CpuPowerCandidateEvidence> {
    if input.observation.objective_signals.cpu_power_limited != Some(true) {
        return None;
    }
    let power_source = &input.system_context.inventory.power_source;
    if power_source.on_battery_or_discharging() && !input.daemon_policy.allow_cpu_power_on_battery {
        return None;
    }

    let limited_cpu = input.observation.objective_signals.cpu_power_limited_cpu?;
    let policy = input
        .system_context
        .inventory
        .cpu_policies
        .iter()
        .find(|policy| parse_related_cpus(policy.related_cpus.as_deref()).contains(&limited_cpu))?;

    let related_cpus = parse_related_cpus(policy.related_cpus.as_deref());
    if related_cpus.is_empty() {
        return None;
    }

    // EPP-only autonomy: the governor is neither required nor changed, so the only question is
    // whether the preference already has the desired value.
    if policy.energy_performance_preference.as_deref() == Some("performance") {
        return None;
    }

    let thermal_headroom = input.observation.objective_signals.thermal_degraded != Some(true)
        && input.system_health.ok_for_apply;
    if !thermal_headroom {
        return None;
    }

    Some(CpuPowerCandidateEvidence {
        policy: policy.policy.clone(),
        related_cpus,
        current_governor: policy.scaling_governor.clone(),
        current_epp: policy.energy_performance_preference.clone(),
        thermal_headroom,
        ac_power: power_source.ac_power_for_evidence(),
        battery_present: power_source.battery_present,
        battery_discharging: power_source.battery_discharging,
        cpu_power_on_battery_allowed: input.daemon_policy.allow_cpu_power_on_battery,
    })
}

fn supports_token(value: Option<&str>, token: &str) -> bool {
    value
        .unwrap_or_default()
        .split_whitespace()
        .any(|part| part == token)
}

fn cpu_power_confidence(
    input: &CandidateProviderInput<'_>,
    evidence: &CpuPowerCandidateEvidence,
) -> f32 {
    let completeness = [
        true,
        !evidence.related_cpus.is_empty(),
        evidence.current_governor.is_some(),
        evidence.current_epp.is_some(),
        evidence.thermal_headroom,
        input
            .observation
            .objective_signals
            .cpu_power_limited_cpu
            .is_some(),
        evidence.power_source_evidence_present(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 7.0;

    let signal_weight = signal_quality_confidence_weight(
        input.observation.objective_signals.signal_quality.cpu_power,
    );

    (input.observation.situation.confidence * completeness * signal_weight).clamp(0.0, 1.0)
}

impl CpuPowerCandidateEvidence {
    fn power_source_evidence_present(&self) -> bool {
        self.ac_power.is_some() || self.battery_discharging.is_some() || !self.battery_present
    }
}

fn parse_related_cpus(related_cpus: Option<&str>) -> Vec<u32> {
    let mut cpus = related_cpus
        .unwrap_or_default()
        .split_whitespace()
        .flat_map(|part| part.split(','))
        .flat_map(parse_cpu_token)
        .collect::<Vec<_>>();
    cpus.sort_unstable();
    cpus.dedup();
    cpus
}

fn parse_cpu_token(token: &str) -> Vec<u32> {
    if let Some((start, end)) = token.split_once('-') {
        let Some(start) = start.parse::<u32>().ok() else {
            return Vec::new();
        };
        let Some(end) = end.parse::<u32>().ok() else {
            return Vec::new();
        };
        return (start..=end).collect();
    }

    token.parse::<u32>().ok().into_iter().collect()
}

#[cfg(test)]
mod tests;
