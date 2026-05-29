use std::{
    fmt, fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::Context;

use crate::{
    actions::runner::ActionRunPolicy,
    autotune::{
        apply::{executor_for_apply_candidate, executor_for_candidate_preview},
        planning::{
            candidate::{ApplyEligibility, try_promote_to_apply_candidate},
            dry_run::CandidateDryRunRecord,
        },
    },
    daemon_policy::{ActionDescriptor, PolicyIntent},
};
#[cfg(test)]
use crate::{
    actions::{RollbackToken, TaskIdentity},
    autotune::planning::candidate::CandidateAction,
    daemon_policy::{DaemonPolicy, DaemonPolicyContext},
};

mod allowlist;
mod audit;
mod error;
mod in_process;
mod model;
mod revalidate;
mod socket;
mod worker;
pub use allowlist::PrivilegeCommandAllowlist;
pub(crate) use audit::{CandidateBoundaryAuditInput, PrivilegeBoundaryAuditInput};
pub use audit::{PrivilegeAuditSink, privileged_operation_audit_event};
pub(crate) use error::PrivilegedWorkerError;
pub use in_process::InProcessPrivilegedActionService;
pub use model::*;
pub(crate) use revalidate::revalidate_candidate_targets;
pub use socket::UnixSocketPrivilegedActionService;
pub use worker::{
    PrivilegedWorkerHandle, default_privileged_worker_socket_path, run_privileged_worker,
    run_privileged_worker_with_service,
};
#[cfg(test)]
pub(crate) use worker::{
    execute_privileged_worker_request, execute_privileged_worker_request_with_audit_sink,
    wait_for_privileged_worker_socket_with_timing,
};

pub trait PrivilegedActionService: fmt::Debug + Send + Sync {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult>;
    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult>;
}

fn stable_error_reason_code(message: &str) -> String {
    message
        .split_once(':')
        .map(|(code, _)| code.trim())
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        })
        .unwrap_or("privileged_worker_request_failed")
        .to_owned()
}

mod serde_u128_string {
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

fn validate_candidate_plan_request(
    request: &CandidateApplyRequest,
    intent: PolicyIntent,
) -> anyhow::Result<()> {
    let now = crate::audit::unix_nanos_now();
    let age_ns = now.saturating_sub(request.plan.created_unix_nanos);

    if request.max_plan_age_nanos > 0 && age_ns > request.max_plan_age_nanos {
        return Err(PrivilegedWorkerError::StaleCandidatePlan {
            age_ns,
            max_ns: request.max_plan_age_nanos,
        }
        .into());
    }

    let live_descriptor = request.plan.candidate.descriptor();
    if live_descriptor.action_id != request.plan.descriptor.action_id
        || live_descriptor.action_kind != request.plan.descriptor.action_kind
        || live_descriptor.safety_class != request.plan.descriptor.safety_class
        || live_descriptor.effect_scope != request.plan.descriptor.effect_scope
    {
        return Err(PrivilegedWorkerError::CandidatePlanDescriptorMismatch.into());
    }

    if request.plan.objective != request.plan.candidate.objective() {
        return Err(PrivilegedWorkerError::CandidatePlanObjectiveMismatch.into());
    }

    if request.plan.evidence_count == 0
        && request.plan.candidate.evidence().is_empty()
        && request.plan.candidate.action_kind() != "cpu_affinity_profile"
    {
        return Err(PrivilegedWorkerError::CandidatePlanMissingEvidence.into());
    }

    request
        .policy
        .check_action_with_context(intent, &request.plan.descriptor, &request.context)?;

    Ok(())
}

#[cfg(test)]
#[path = "privilege/tests/mod.rs"]
mod tests;
