use super::model::{IrqAffinityAction, IrqAffinityPolicy, IrqAffinityRisk};
use crate::actions::ActionBoundaryError;

pub(super) fn validate_policy_and_request(
    action: &IrqAffinityAction,
    policy: &IrqAffinityPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_irq_affinity_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "irq_affinity",
            requirement: "allow_irq_affinity_changes",
        }
        .into());
    }

    if action.irq == 0 {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "irq_affinity",
            field: "irq".to_owned(),
            reason: "IRQ number must be greater than zero".to_owned(),
        }
        .into());
    }

    if action.device_hint.trim().is_empty() {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "irq_affinity",
            field: "device_hint".to_owned(),
            reason: "IRQ device mapping must be known before affinity changes".to_owned(),
        }
        .into());
    }

    if matches!(action.risk, IrqAffinityRisk::HighRisk) && !policy.allow_high_risk_devices {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "irq_affinity",
            requirement: "allow_high_risk_devices",
        }
        .into());
    }

    if policy.require_strong_irq_evidence && !action.evidence.strong_irq_evidence {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "irq_affinity",
            requirement: "strong_irq_evidence",
        }
        .into());
    }

    if policy.require_stable_irq_identity && !action.evidence.stable_irq_identity {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "irq_affinity",
            requirement: "stable_irq_identity",
        }
        .into());
    }

    if policy.require_known_device_mapping && !action.evidence.known_device_mapping {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "irq_affinity",
            requirement: "known_device_mapping",
        }
        .into());
    }

    if let Some(observed_irq) = action.evidence.observed_irq
        && observed_irq != action.irq
    {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "irq_affinity",
            reason: format!(
                "IRQ evidence mismatch: action irq={} observed irq={}",
                action.irq, observed_irq
            ),
        }
        .into());
    }

    if let Some(observed_device_hint) = &action.evidence.observed_device_hint
        && observed_device_hint != &action.device_hint
    {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "irq_affinity",
            reason: format!(
                "IRQ device evidence mismatch: action device={:?} observed device={:?}",
                action.device_hint, observed_device_hint
            ),
        }
        .into());
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
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "irq_affinity",
            reason: format!(
                "refusing to change system-critical IRQ {} ({})",
                action.irq, action.device_hint
            ),
        }
        .into());
    }

    Ok(())
}

pub(super) fn validate_affinity_value(value: &str) -> anyhow::Result<()> {
    let value = value.trim();

    if value.is_empty() {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "irq_affinity",
            field: "smp_affinity".to_owned(),
            reason: "smp_affinity must not be empty".to_owned(),
        }
        .into());
    }

    if value.len() > 256 {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "irq_affinity",
            field: "smp_affinity".to_owned(),
            reason: "smp_affinity is too long".to_owned(),
        }
        .into());
    }

    for ch in value.chars() {
        if !(ch.is_ascii_hexdigit() || ch == ',') {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "irq_affinity",
                field: "smp_affinity".to_owned(),
                reason: format!("smp_affinity contains invalid character {ch:?}"),
            }
            .into());
        }
    }

    if value.split(',').any(|part| part.is_empty()) {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "irq_affinity",
            field: "smp_affinity".to_owned(),
            reason: "smp_affinity contains an empty comma-separated component".to_owned(),
        }
        .into());
    }

    Ok(())
}
