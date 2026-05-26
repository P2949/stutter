use super::{GeneratedCpuSetPolicy, helpers::*};
use crate::{
    profiles::{Profile, ProfileRule},
    topology::{TopologyModel, cpu_mask_to_vec},
};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GeneratedProfileInvariants {
    pub render_has_cpu: bool,
    pub compositor_has_cpu: bool,
    pub background_capacity_ok: bool,
    pub no_empty_masks: bool,
}

impl GeneratedProfileInvariants {
    pub fn check(profile: &Profile, policy: &GeneratedCpuSetPolicy) -> Self {
        let mut render_has_cpu = true;
        let mut compositor_has_cpu = true;
        let mut background_capacity_ok = true;
        let mut no_empty_masks = true;

        for rule in &profile.rules {
            if let Some(affinity) = rule.affinity.as_ref() {
                if affinity.is_empty() {
                    no_empty_masks = false;
                }

                let cpu_count = cpu_mask_to_vec(affinity).len();

                if rule_matches_render_or_main_game(rule) && cpu_count < policy.min_render_cpus {
                    render_has_cpu = false;
                }
                if rule_matches_game_work(rule) && cpu_count < policy.min_game_cpus {
                    render_has_cpu = false;
                }
                if rule_matches_compositor_or_gamescope(rule)
                    && cpu_count < policy.min_compositor_cpus
                {
                    compositor_has_cpu = false;
                }

                if profile.name != "baseline-online"
                    && rule_matches_background_or_helper_work(rule)
                    && cpu_count < policy.min_background_cpus
                {
                    background_capacity_ok = false;
                }
            } else {
                no_empty_masks = false;
            }
        }

        Self {
            render_has_cpu,
            compositor_has_cpu,
            background_capacity_ok,
            no_empty_masks,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.render_has_cpu
            && self.compositor_has_cpu
            && self.background_capacity_ok
            && self.no_empty_masks
    }

    pub fn to_rejection_reason(&self) -> String {
        let mut reasons = Vec::new();
        if !self.no_empty_masks {
            reasons.push("contains empty CPU masks");
        }
        if !self.render_has_cpu {
            reasons.push("render/main game missing minimum CPUs");
        }
        if !self.compositor_has_cpu {
            reasons.push("compositor/gamescope missing minimum CPUs");
        }
        if !self.background_capacity_ok {
            reasons.push("background/helper capacity too low");
        }
        reasons.join(", ")
    }
}

pub(crate) fn validate_generated_profile(
    profile: &Profile,
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    if profile.rules.is_empty() {
        return Err("generated profile has no rules".to_owned());
    }

    let online = &topology.online_cpus;

    for (index, rule) in profile.rules.iter().enumerate() {
        validate_generated_rule_mask(profile, index, rule, online, policy)?;
    }

    let invariants = GeneratedProfileInvariants::check(profile, policy);
    if !invariants.is_valid() {
        return Err(invariants.to_rejection_reason());
    }

    Ok(())
}

pub(crate) fn validate_generated_rule_mask(
    profile: &Profile,
    index: usize,
    rule: &ProfileRule,
    online: &crate::affinity::CpuMask,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    let Some(affinity) = rule.affinity.as_ref() else {
        return Err(format!("rule {index} is missing CPU mask"));
    };

    if affinity.is_empty() {
        return Err(format!("rule {index} has empty CPU mask"));
    }

    if !affinity.is_subset_of(online) {
        return Err(format!(
            "rule {index} requests offline CPUs: requested={} online={}",
            affinity.to_range_string(),
            online.to_range_string()
        ));
    }

    if let Some(allowed) = &policy.allowed_cpus
        && !affinity.is_subset_of(allowed)
    {
        return Err(format!(
            "rule {index} violates allowed CPU set: requested={} allowed={}",
            affinity.to_range_string(),
            allowed.to_range_string()
        ));
    }

    if let Some(denied) = &policy.denied_cpus {
        let requested = cpu_mask_to_vec(affinity);
        let denied = cpu_mask_to_vec(denied);
        let overlap = requested
            .into_iter()
            .filter(|cpu| denied.contains(cpu))
            .collect::<Vec<_>>();

        if !overlap.is_empty() {
            return Err(format!(
                "rule {index} violates denied CPU set: requested={} denied={} overlap={}",
                affinity.to_range_string(),
                policy
                    .denied_cpus
                    .as_ref()
                    .map(|mask| mask.to_range_string())
                    .unwrap_or_default(),
                crate::topology::cpus_to_range_string(&overlap)
            ));
        }
    }

    if profile.name != "baseline-online" && rule_matches_audio_or_input(rule) {
        return Err(format!(
            "rule {index} targets critical realtime/input classes in generated profile"
        ));
    }

    Ok(())
}
