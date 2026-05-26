use super::model::{IrqAffinityAction, IrqAffinityPolicy, IrqAffinityRisk};

pub(super) fn validate_policy_and_request(
    action: &IrqAffinityAction,
    policy: &IrqAffinityPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_irq_affinity_changes {
        anyhow::bail!(
            "policy does not allow IRQ affinity changes; advisor remains investigate-first"
        );
    }

    if action.irq == 0 {
        anyhow::bail!("IRQ number must be greater than zero");
    }

    if action.device_hint.trim().is_empty() {
        anyhow::bail!("IRQ device mapping must be known before affinity changes");
    }

    if matches!(action.risk, IrqAffinityRisk::HighRisk) && !policy.allow_high_risk_devices {
        anyhow::bail!("policy does not allow high-risk IRQ affinity changes");
    }

    if policy.require_strong_irq_evidence && !action.evidence.strong_irq_evidence {
        anyhow::bail!("strong IRQ evidence is required before changing IRQ affinity");
    }

    if policy.require_stable_irq_identity && !action.evidence.stable_irq_identity {
        anyhow::bail!("stable IRQ identity is required before changing IRQ affinity");
    }

    if policy.require_known_device_mapping && !action.evidence.known_device_mapping {
        anyhow::bail!("known device mapping is required before changing IRQ affinity");
    }

    if let Some(observed_irq) = action.evidence.observed_irq
        && observed_irq != action.irq
    {
        anyhow::bail!(
            "IRQ evidence mismatch: action irq={} observed irq={}",
            action.irq,
            observed_irq
        );
    }

    if let Some(observed_device_hint) = &action.evidence.observed_device_hint
        && observed_device_hint != &action.device_hint
    {
        anyhow::bail!(
            "IRQ device evidence mismatch: action device={:?} observed device={:?}",
            action.device_hint,
            observed_device_hint
        );
    }

    Ok(())
}

pub(super) fn validate_irq_identity(action: &IrqAffinityAction) -> anyhow::Result<()> {
    if action.device_hint.to_ascii_lowercase().contains("timer")
        || action
            .device_hint
            .to_ascii_lowercase()
            .contains("rescheduling")
        || action
            .device_hint
            .to_ascii_lowercase()
            .contains("call function")
        || action.device_hint.to_ascii_lowercase().contains("ipi")
    {
        anyhow::bail!(
            "refusing to change system-critical IRQ {} ({})",
            action.irq,
            action.device_hint
        );
    }

    Ok(())
}

pub(super) fn validate_affinity_value(value: &str) -> anyhow::Result<()> {
    let value = value.trim();

    if value.is_empty() {
        anyhow::bail!("smp_affinity must not be empty");
    }

    if value.len() > 256 {
        anyhow::bail!("smp_affinity is too long");
    }

    for ch in value.chars() {
        if !(ch.is_ascii_hexdigit() || ch == ',') {
            anyhow::bail!("smp_affinity contains invalid character {ch:?}");
        }
    }

    if value.split(',').any(|part| part.is_empty()) {
        anyhow::bail!("smp_affinity contains an empty comma-separated component");
    }

    Ok(())
}
