use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::*;
use crate::autotune::{
    external_mutation::ExternalMutationPolicy, workload_policy::DaemonWorkloadPolicyConfig,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonCandidateConfidenceConfig {
    pub min_suggest_confidence: f32,
    pub min_apply_low_risk_confidence: f32,
    pub min_apply_medium_risk_confidence: f32,
    pub min_high_risk_suggestion_confidence: f32,
}

impl Default for DaemonCandidateConfidenceConfig {
    fn default() -> Self {
        Self {
            min_suggest_confidence: 0.50,
            min_apply_low_risk_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            min_apply_medium_risk_confidence: 0.85,
            min_high_risk_suggestion_confidence: 0.90,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonAutotuneConfig {
    pub candidate_window_seconds: u64,
    pub washout_seconds: u64,
    pub rollback_on_crash_recovery: bool,
    pub allow_medium_risk_apply: bool,
    pub allow_cpu_power_on_battery: bool,
    #[serde(default)]
    pub allow_gpu_power_in_autotune: bool,
    #[serde(default)]
    pub allow_vm_knobs_in_autotune: bool,
    pub privileged_worker_socket: Option<PathBuf>,
    pub unsafe_in_process_privileged_worker: bool,
    #[serde(default = "default_manage_privileged_worker")]
    pub manage_privileged_worker: bool,
    #[serde(default = "default_privileged_worker_restart_limit")]
    pub privileged_worker_restart_limit: u32,
    #[serde(default = "default_privileged_worker_socket_ready_timeout_ms")]
    pub privileged_worker_socket_ready_timeout_ms: u64,
    #[serde(default = "default_privileged_worker_socket_ready_retry_ms")]
    pub privileged_worker_socket_ready_retry_ms: u64,
    #[serde(default = "default_privileged_worker_shutdown_poll_ms")]
    pub privileged_worker_shutdown_poll_ms: u64,
    #[serde(default)]
    pub external_mutation_policy: ExternalMutationPolicy,
    #[serde(default)]
    pub high_risk_dry_run: bool,
    pub workload_policy: DaemonWorkloadPolicyConfig,
    pub confidence: DaemonCandidateConfidenceConfig,
}

impl Default for DaemonAutotuneConfig {
    fn default() -> Self {
        Self {
            candidate_window_seconds: 30,
            washout_seconds: 10,
            rollback_on_crash_recovery: true,
            allow_medium_risk_apply: false,
            allow_cpu_power_on_battery: false,
            allow_gpu_power_in_autotune: false,
            allow_vm_knobs_in_autotune: false,
            privileged_worker_socket: None,
            unsafe_in_process_privileged_worker: false,
            manage_privileged_worker: default_manage_privileged_worker(),
            privileged_worker_restart_limit: default_privileged_worker_restart_limit(),
            privileged_worker_socket_ready_timeout_ms:
                default_privileged_worker_socket_ready_timeout_ms(),
            privileged_worker_socket_ready_retry_ms:
                default_privileged_worker_socket_ready_retry_ms(),
            privileged_worker_shutdown_poll_ms: default_privileged_worker_shutdown_poll_ms(),
            external_mutation_policy: ExternalMutationPolicy::default(),
            high_risk_dry_run: false,
            workload_policy: DaemonWorkloadPolicyConfig::default(),
            confidence: DaemonCandidateConfidenceConfig::default(),
        }
    }
}

fn default_manage_privileged_worker() -> bool {
    true
}

fn default_privileged_worker_restart_limit() -> u32 {
    3
}

fn default_privileged_worker_socket_ready_timeout_ms() -> u64 {
    DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_TIMEOUT_MS
}

fn default_privileged_worker_socket_ready_retry_ms() -> u64 {
    DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_RETRY_MS
}

fn default_privileged_worker_shutdown_poll_ms() -> u64 {
    DEFAULT_PRIVILEGED_WORKER_SHUTDOWN_POLL_MS
}
