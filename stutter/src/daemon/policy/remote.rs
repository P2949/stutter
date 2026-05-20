//! Remote daemon policy state and rule recording; this module does not own local safety gates.

use serde::{Deserialize, Serialize};

use super::{
    DaemonMode, DaemonPolicy, HIGH_RISK_APPLY_IMPLEMENTED, PolicyRejection, RemotePolicyContext,
    build::max_safety_class_for_mode, evaluate::record_policy_rule,
};
use crate::daemon::{config::DaemonConfig, explain::PolicyRuleEvaluation};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteApplyPolicy {
    pub allow_remote_apply: bool,
    pub remote_apply_enabled: bool,
    pub require_loopback_bind: bool,
    pub require_auth: bool,
    pub max_remote_targets: usize,
    pub target_count: usize,
    pub target_count_allowed: bool,
    pub mode_supported_by_limits: bool,
    pub auth_configured: bool,
    pub request_authorized: bool,
    pub bind_is_loopback: bool,
}

impl Default for RemoteApplyPolicy {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            remote_apply_enabled: false,
            require_loopback_bind: true,
            require_auth: true,
            max_remote_targets: 1,
            target_count: 0,
            target_count_allowed: true,
            mode_supported_by_limits: false,
            auth_configured: false,
            request_authorized: false,
            bind_is_loopback: false,
        }
    }
}

pub(in crate::daemon::policy) fn remote_apply_policy_for_config(
    config: &DaemonConfig,
    remote_context: Option<&RemotePolicyContext>,
) -> RemoteApplyPolicy {
    let Some(context) = remote_context else {
        return RemoteApplyPolicy::default();
    };

    let mode_supported_by_limits = remote_mode_supported_by_context(config.mode, context);
    let auth_allowed = !config.remote.require_auth_for_apply
        || (context.auth_configured && context.request_authorized);
    let bind_allowed = config.remote.allow_non_loopback_apply || context.bind_is_loopback;
    let target_count = remote_target_count_for_config(config);
    let target_count_allowed = target_count <= context.limits.max_targets;

    RemoteApplyPolicy {
        allow_remote_apply: config.remote.allow_remote_apply
            && config.mode.supports_apply()
            && mode_supported_by_limits
            && auth_allowed
            && bind_allowed
            && target_count_allowed,
        remote_apply_enabled: config.remote.allow_remote_apply,
        require_loopback_bind: config.mode.supports_apply()
            && !config.remote.allow_non_loopback_apply,
        require_auth: config.mode.supports_apply() && config.remote.require_auth_for_apply,
        max_remote_targets: context.limits.max_targets,
        target_count,
        target_count_allowed,
        mode_supported_by_limits,
        auth_configured: context.auth_configured,
        request_authorized: context.request_authorized,
        bind_is_loopback: context.bind_is_loopback,
    }
}

fn remote_mode_supported_by_context(mode: DaemonMode, context: &RemotePolicyContext) -> bool {
    let mode_max_safety_class = max_safety_class_for_mode(mode);

    mode <= context.limits.max_mode
        && mode_max_safety_class <= context.limits.max_safety_class
        && (mode != DaemonMode::ApplyHighRisk
            || (HIGH_RISK_APPLY_IMPLEMENTED && context.limits.allow_high_risk))
}

fn remote_target_count_for_config(config: &DaemonConfig) -> usize {
    config.target.target_pids.len()
        + config.target.tree_pids.len()
        + usize::from(config.target.watch_process.is_some())
}

pub(in crate::daemon::policy) fn record_remote_target_count_rule(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    let remote = &policy.remote_apply;

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_target_count",
        remote.target_count_allowed,
        if remote.target_count_allowed {
            format!(
                "remote target count {} is within max_targets {}",
                remote.target_count, remote.max_remote_targets
            )
        } else {
            format!(
                "remote target count {} exceeds max_targets {}",
                remote.target_count, remote.max_remote_targets
            )
        },
        (!remote.target_count_allowed).then_some(PolicyRejection::RemoteTargetCountTooHigh {
            target_count: remote.target_count,
            max_targets: remote.max_remote_targets,
        }),
    );
}

pub(in crate::daemon::policy) fn record_remote_apply_rules(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    let remote = &policy.remote_apply;

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_apply_enabled",
        remote.remote_apply_enabled,
        if remote.remote_apply_enabled {
            "remote apply is enabled by daemon configuration".to_owned()
        } else {
            "remote apply is disabled by daemon configuration".to_owned()
        },
        (!remote.remote_apply_enabled).then_some(PolicyRejection::RemoteApplyDisabled),
    );

    if remote.require_auth {
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_auth_configured",
            remote.auth_configured,
            if remote.auth_configured {
                "remote apply has configured bearer-token authentication".to_owned()
            } else {
                "remote apply requires configured bearer-token authentication".to_owned()
            },
            (!remote.auth_configured).then_some(PolicyRejection::RemoteApplyRequiresConfiguredAuth),
        );

        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_request_authorized",
            remote.request_authorized,
            if remote.request_authorized {
                "remote apply request is authorized".to_owned()
            } else {
                "remote apply request is not authorized".to_owned()
            },
            (!remote.request_authorized)
                .then_some(PolicyRejection::RemoteApplyRequiresAuthorizedRequest),
        );
    }

    if remote.require_loopback_bind {
        record_policy_rule(
            evaluated_rules,
            first_rejection,
            "remote_loopback_bind",
            remote.bind_is_loopback,
            if remote.bind_is_loopback {
                "remote apply bind address is loopback".to_owned()
            } else {
                "remote apply bind address is not loopback".to_owned()
            },
            (!remote.bind_is_loopback).then_some(PolicyRejection::RemoteApplyRequiresLoopbackBind),
        );
    }

    record_policy_rule(
        evaluated_rules,
        first_rejection,
        "remote_mode_supported",
        remote.mode_supported_by_limits,
        if remote.mode_supported_by_limits {
            format!(
                "remote mode {} is within configured remote limits",
                policy.mode
            )
        } else {
            format!(
                "remote mode {} exceeds configured remote limits",
                policy.mode
            )
        },
        (!remote.mode_supported_by_limits)
            .then_some(PolicyRejection::RemoteModeNotAllowed { mode: policy.mode }),
    );

    record_remote_target_count_rule(policy, evaluated_rules, first_rejection);
}
