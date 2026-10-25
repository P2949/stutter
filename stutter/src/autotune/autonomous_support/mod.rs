//! The autonomous support registry and the runtime gate that consumes it. Normal autonomous apply
//! requires, together: a Supported objective that allows autonomous apply, a Supported product
//! capability for the operation, and an exact registry contract for the operation, workload,
//! pressure, objective and effect scope. Anything absent is suggest-only, benchmark-only, manual
//! or rejected. Machine capability, workload policy, health and conflicts are checked elsewhere;
//! this gate adds the product-maturity half.
//!
//! The registry is the single source of truth: documentation and capability output derive from it.
use serde::{Deserialize, Serialize};

use crate::{
    autotune::{
        objective::{ObjectiveKind, ObjectiveMaturity},
        operation::MutationOperationKind,
        situation::SituationKind,
        situation_model::{PressureKind, WorkloadKind},
    },
    capability_tiers::{CAPABILITIES, CapabilityTier},
    daemon_policy::{ActionEffectScope, ActionSource},
};

/// One exact autonomous contract. A Supported tuple authorises nothing beyond itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutonomousContract {
    pub operation: MutationOperationKind,
    pub workload: WorkloadKind,
    pub pressure: PressureKind,
    pub objective: ObjectiveKind,
    pub effect_scope: ActionEffectScope,
    pub prerequisites: &'static [&'static str],
}

/// Every Supported autonomous contract. An addition comes with the exact-commit evidence of its
/// graduation campaign (`xtask mutation-campaign`) and nothing here widens beyond that tuple.
pub const AUTONOMOUS_CONTRACTS: &[AutonomousContract] = &[
    // M4: CPU affinity for a stable, explicitly identified game under scheduler pressure, judged
    // by the target's own runnable latency.
    AutonomousContract {
        operation: MutationOperationKind::CpuAffinityProfile,
        workload: WorkloadKind::Game,
        pressure: PressureKind::Scheduler,
        objective: ObjectiveKind::GameRunnableLatency,
        effect_scope: ActionEffectScope::LocalProcessTree,
        prerequisites: &[
            "explicit_or_stable_game_target",
            "baseline_runnable_population_of_at_least_1000_samples",
            "prepared_rollback_token_durable_before_first_write",
        ],
    },
];

/// Authorises exactly one operation and objective for one validation campaign. It never
/// authorises other operations or objectives, remote callers, or high-risk operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationOverride {
    pub operation: MutationOperationKind,
    pub objective: ObjectiveKind,
    pub campaign_id: String,
}

impl ValidationOverride {
    /// Whether this override is the campaign's own exact operation and objective, held by a local
    /// caller. Only such a candidate may bypass the per-workload autonomous-family table: the
    /// campaign is the evidence that decides whether the table may ever list it.
    pub fn covers(
        &self,
        operation: MutationOperationKind,
        objective: ObjectiveKind,
        source: ActionSource,
    ) -> bool {
        source != ActionSource::RemoteAgent
            && self.operation == operation
            && self.objective == objective
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutonomousGateInput<'a> {
    pub operation: MutationOperationKind,
    pub objective: ObjectiveKind,
    pub situation: SituationKind,
    pub effect_scope: ActionEffectScope,
    pub source: ActionSource,
    pub validation_override: Option<&'a ValidationOverride>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutonomousDenial {
    /// Unclassified, manual-only or HighRisk operations are never autonomous.
    OperationNotAutonomous,
    ObjectiveNotSupported(ObjectiveMaturity),
    ObjectiveAutonomousApplyNotAllowed,
    CapabilityTierNotSupported,
    /// The operation has no prepared-before-mutation transaction yet.
    PreparedTransactionMissing,
    NotInSupportRegistry,
    /// A validation campaign owns the run: only its own operation may be applied.
    OutsideValidationScope,
}

impl AutonomousDenial {
    pub fn deny_reason(self) -> crate::autotune::planning::denial::CandidateDenyReason {
        use crate::autotune::planning::denial::CandidateDenyReason as Reason;
        match self {
            Self::OperationNotAutonomous => Reason::OperationNotAutonomous,
            Self::ObjectiveNotSupported(_) => Reason::ObjectiveNotSupported,
            Self::ObjectiveAutonomousApplyNotAllowed => Reason::ObjectiveAutonomousApplyNotAllowed,
            Self::CapabilityTierNotSupported => Reason::CapabilityTierNotSupported,
            Self::PreparedTransactionMissing => Reason::PreparedTransactionMissing,
            Self::NotInSupportRegistry => Reason::NotInSupportRegistry,
            Self::OutsideValidationScope => Reason::OutsideValidationScope,
        }
    }

    pub fn reason_code(self) -> &'static str {
        match self {
            Self::OperationNotAutonomous => "operation_not_autonomous",
            Self::ObjectiveNotSupported(_) => "objective_not_supported",
            Self::ObjectiveAutonomousApplyNotAllowed => "objective_autonomous_apply_not_allowed",
            Self::CapabilityTierNotSupported => "capability_tier_not_supported",
            Self::PreparedTransactionMissing => "prepared_transaction_missing",
            Self::NotInSupportRegistry => "operation_not_in_support_registry",
            Self::OutsideValidationScope => "outside_validation_scope",
        }
    }

    pub fn message(self, input: &AutonomousGateInput<'_>) -> String {
        match self {
            Self::OperationNotAutonomous => format!(
                "operation {} is never applied autonomously",
                input.operation.as_str()
            ),
            Self::ObjectiveNotSupported(maturity) => format!(
                "objective {:?} is {:?}; autonomous apply requires Supported",
                input.objective, maturity
            ),
            Self::ObjectiveAutonomousApplyNotAllowed => format!(
                "objective {:?} does not allow autonomous apply",
                input.objective
            ),
            Self::CapabilityTierNotSupported => format!(
                "product capability for operation {} is not Supported",
                input.operation.as_str()
            ),
            Self::PreparedTransactionMissing => format!(
                "operation {} has no prepared-before-mutation transaction",
                input.operation.as_str()
            ),
            Self::OutsideValidationScope => format!(
                "operation {} is outside the validation campaign's scope",
                input.operation.as_str()
            ),
            Self::NotInSupportRegistry => format!(
                "no Supported contract covers operation={} workload={:?} pressure={:?} objective={:?} scope={:?}",
                input.operation.as_str(),
                input.situation.workload(),
                input.situation.pressure(),
                input.objective,
                input.effect_scope
            ),
        }
    }
}

fn capability_supported(operation: MutationOperationKind) -> bool {
    operation.capability_id().is_some_and(|id| {
        CAPABILITIES
            .iter()
            .any(|capability| capability.id == id && capability.tier == CapabilityTier::Supported)
    })
}

fn registry_allows(input: &AutonomousGateInput<'_>, contracts: &[AutonomousContract]) -> bool {
    let (workload, pressure) = (input.situation.workload(), input.situation.pressure());
    contracts.iter().any(|contract| {
        contract.operation == input.operation
            && contract.objective == input.objective
            && contract.effect_scope == input.effect_scope
            && contract.workload == workload
            && Some(contract.pressure) == pressure
    })
}

/// The product-maturity gate for normal autonomous apply.
pub fn autonomous_apply_gate(input: &AutonomousGateInput<'_>) -> Result<(), AutonomousDenial> {
    let descriptor = input.objective.descriptor();
    autonomous_apply_gate_with(
        input,
        GateFacts {
            maturity: descriptor.maturity,
            autonomous_apply_allowed: descriptor.autonomous_apply_allowed,
            capability_supported: capability_supported(input.operation),
            contracts: AUTONOMOUS_CONTRACTS,
        },
    )
}

/// The facts the gate decides from; tests substitute them to exercise a Supported tuple.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateFacts<'a> {
    pub maturity: ObjectiveMaturity,
    pub autonomous_apply_allowed: bool,
    pub capability_supported: bool,
    pub contracts: &'a [AutonomousContract],
}

pub(crate) fn autonomous_apply_gate_with(
    input: &AutonomousGateInput<'_>,
    facts: GateFacts<'_>,
) -> Result<(), AutonomousDenial> {
    if input.operation.is_manual_only() {
        return Err(AutonomousDenial::OperationNotAutonomous);
    }

    // A validation campaign may exercise exactly its own operation/objective, only through a
    // local caller, and only for an objective that is at least Validating. While it holds the
    // run, no other operation is applied: a measurement attributes its effect to one mutation.
    if let Some(grant) = input.validation_override
        && input.source != ActionSource::RemoteAgent
    {
        if grant.operation != input.operation {
            return Err(AutonomousDenial::OutsideValidationScope);
        }
        if grant.objective == input.objective {
            return match facts.maturity {
                ObjectiveMaturity::Experimental => {
                    Err(AutonomousDenial::ObjectiveNotSupported(facts.maturity))
                }
                ObjectiveMaturity::Validating | ObjectiveMaturity::Supported => Ok(()),
            };
        }
    }

    if facts.maturity != ObjectiveMaturity::Supported {
        return Err(AutonomousDenial::ObjectiveNotSupported(facts.maturity));
    }
    if !facts.autonomous_apply_allowed {
        return Err(AutonomousDenial::ObjectiveAutonomousApplyNotAllowed);
    }
    if !facts.capability_supported {
        return Err(AutonomousDenial::CapabilityTierNotSupported);
    }
    if !input.operation.prepared_transaction_implemented() {
        return Err(AutonomousDenial::PreparedTransactionMissing);
    }
    if !registry_allows(input, facts.contracts) {
        return Err(AutonomousDenial::NotInSupportRegistry);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
