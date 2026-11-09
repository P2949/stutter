//! `PmQosCpuLatency` as a manual, policy-gated action (package W11d).
//!
//! The lease is held by this process under an enforced hold deadline (`pm_qos_watchdog`); the
//! rollback token names it so recovery can prove it is gone. Like the other HighRisk actions it is
//! never an executable autotune plan and never autonomous: the default policy denies, and applying
//! needs an explicit `allow` plus the lease policy's floor, ceiling and power guards.
use std::{path::PathBuf, sync::Mutex};

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    pm_qos::{PmQosPolicy, PowerContext},
    pm_qos_rollback::check_release,
    pm_qos_transaction::{PmQosReleaseRecord, prepare},
    pm_qos_watchdog::{PmQosWatchdog, enforce_deadline},
};

pub const PM_QOS_DEVICE: &str = "/dev/cpu_dma_latency";

#[derive(Clone, Debug)]
pub struct PmQosActionPolicy {
    /// Nothing is requested unless this is set explicitly.
    pub allow: bool,
    pub lease: PmQosPolicy,
    /// Fresh power facts from the CPU power sensor at apply time.
    pub power: PowerContext,
}

impl Default for PmQosActionPolicy {
    fn default() -> Self {
        Self {
            allow: false,
            lease: PmQosPolicy::default(),
            power: PowerContext {
                on_ac: None,
                throttle_events_recent: None,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct PmQosCpuLatencyAction {
    pub device: PathBuf,
    pub latency_us: u32,
}

impl PmQosCpuLatencyAction {
    pub fn new(latency_us: u32) -> Self {
        Self {
            device: PathBuf::from(PM_QOS_DEVICE),
            latency_us,
        }
    }
}

/// Leases this process holds, each under its deadline watchdog.
static HELD: Mutex<Vec<(PmQosReleaseRecord, PmQosWatchdog)>> = Mutex::new(Vec::new());

fn denied(requirement: &'static str) -> anyhow::Error {
    ActionBoundaryError::PolicyDenied {
        action_kind: "pm_qos_cpu_latency",
        requirement,
    }
    .into()
}

impl PmQosCpuLatencyAction {
    fn prepared(
        &self,
        policy: &PmQosActionPolicy,
    ) -> anyhow::Result<super::pm_qos_transaction::PmQosPrepared> {
        if !policy.allow {
            return Err(denied("allow_pm_qos"));
        }
        prepare(&self.device, self.latency_us, &policy.lease, &policy.power)
            .map_err(|error| anyhow::anyhow!("pm_qos_cpu_latency refused: {error:?}"))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &PmQosActionPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy)?;
        Ok(vec![ActionWarning {
            message: "a PM QoS request is system-wide: it keeps every CPU out of deep idle states while held, and is released at its hold limit".to_owned(),
        }])
    }

    pub fn apply_with_policy(&self, policy: &PmQosActionPolicy) -> anyhow::Result<RollbackToken> {
        let held = self
            .prepared(policy)?
            .commit(&policy.power)
            .map_err(|error| anyhow::anyhow!("pm_qos_cpu_latency not in effect: {error:?}"))?;
        let record = held.release_record();
        let watchdog = enforce_deadline(held)?;
        HELD.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push((record.clone(), watchdog));
        Ok(RollbackToken::PmQosRelease {
            records: vec![record],
        })
    }
}

impl TuningAction for PmQosCpuLatencyAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!("pm-qos:cpu-latency:{}us", self.latency_us))
    }

    fn describe(&self) -> String {
        format!(
            "request a {} us system-wide CPU wake latency while a bounded lease is held",
            self.latency_us
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        let action_id = self.id();
        crate::daemon_policy::ActionDescriptor {
            action_kind: "pm_qos_cpu_latency".to_owned(),
            action_id,
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::SystemWide,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: true,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&PmQosActionPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&PmQosActionPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&PmQosActionPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let effective = super::pm_qos::read_effective(&self.device)?;
        Ok(ActionState {
            applied: effective <= self.latency_us,
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(effective > self.latency_us),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::PmQosRelease { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("pm-qos-release"),
            )
            .into());
        };
        // Release the leases this process holds first; recovery then proves the target moved.
        let mut held = HELD.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut index = 0;
        while index < held.len() {
            if records.contains(&held[index].0) {
                held.remove(index).1.release();
            } else {
                index += 1;
            }
        }
        drop(held);
        let check = check_release(records);
        anyhow::ensure!(
            check.unreadable == 0 && check.unproven == 0,
            "PM QoS release not proven: {}",
            check.messages.join("; ")
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "pm_qos_action_tests.rs"]
mod tests;
