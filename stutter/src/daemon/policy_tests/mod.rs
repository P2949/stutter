//! Test modules for `daemon::policy` split by policy decision dimension.
//!
//! Owns daemon policy test module wiring and shared policy test fixtures.
//! Does not own production daemon policy behavior.

mod capabilities;
mod explain;
mod mode;
mod remote;
mod safety;
mod snapshots;

use super::*;
use crate::{actions::ActionId, daemon::capabilities::DaemonCapabilities};

fn descriptor(safety_class: SafetyClass) -> ActionDescriptor {
    descriptor_with(
        safety_class,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::RequiredBeforeApply,
    )
}

fn descriptor_with(
    safety_class: SafetyClass,
    effect_scope: ActionEffectScope,
    rollback: RollbackRequirement,
) -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId::new("test-action".to_owned()),
        action_kind: "test".to_owned(),
        safety_class,
        effect_scope,
        rollback,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: true,
        confidence: Some(0.90),
    }
}

fn all_safety_classes() -> [SafetyClass; 4] {
    [
        SafetyClass::ObserveOnly,
        SafetyClass::ReversibleLowRisk,
        SafetyClass::ReversibleMediumRisk,
        SafetyClass::HighRisk,
    ]
}

fn all_capabilities_available() -> DaemonCapabilities {
    DaemonCapabilities {
        kernel_release: Some("6.9.1-test".to_owned()),
        btf_available: true,
        sched_tracepoints_available: true,
        perf_permissions_likely: true,
        perf_event_paranoid: Some(1),
        cgroup_v2_available: true,
        sched_ext_available: true,
        uclamp_available: true,
        ionice_available: true,
        irq_affinity_available: true,
        gpu_sysfs_available: true,
        privileged_worker_socket_reachable: Some(true),
    }
}

#[test]
fn policy_rejects_empty_action_id_descriptor() {
    use crate::{
        actions::SafetyClass,
        daemon::policy::{
            ActionDescriptor, ActionEffectScope, ActionSource, DaemonPolicy, PolicyIntent,
            PolicyRejection, RollbackRequirement,
        },
    };

    let policy = DaemonPolicy::observe(ActionSource::Test);
    let descriptor = ActionDescriptor {
        action_id: crate::actions::ActionId::new(""),
        action_kind: "test-action".to_owned(),
        safety_class: SafetyClass::ObserveOnly,
        effect_scope: ActionEffectScope::LocalProcessTree,
        rollback: RollbackRequirement::NotRequiredForDryRun,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: false,
        confidence: None,
    };

    let err = policy
        .check_action(PolicyIntent::DryRun, &descriptor)
        .expect_err("empty action_id should be rejected");

    assert!(matches!(
        err,
        PolicyRejection::InvalidActionDescriptor { .. }
    ));
    assert!(
        err.to_string().contains("ActionId cannot be empty"),
        "unexpected error: {err}"
    );
}
