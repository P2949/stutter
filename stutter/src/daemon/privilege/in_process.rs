//! In-process privileged action service implementation.

use super::*;

#[derive(Clone, Debug)]
pub struct InProcessPrivilegedActionService {
    proc_root: PathBuf,
    audit_sink: PrivilegeAuditSink,
}

impl Default for InProcessPrivilegedActionService {
    fn default() -> Self {
        Self::with_proc_root_and_audit_path("/proc", crate::audit::default_audit_log_path())
    }
}

impl InProcessPrivilegedActionService {
    #[cfg(test)]
    pub fn with_proc_root(proc_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            audit_sink: PrivilegeAuditSink::default(),
        }
    }

    #[cfg(test)]
    pub fn with_audit_path(audit_path: impl Into<PathBuf>) -> Self {
        Self {
            audit_sink: PrivilegeAuditSink::to_path(audit_path),
            ..Self::default()
        }
    }

    pub fn with_proc_root_and_audit_path(
        proc_root: impl Into<PathBuf>,
        audit_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            proc_root: proc_root.into(),
            audit_sink: PrivilegeAuditSink::to_path(audit_path),
        }
    }

    fn audit_candidate_boundary(&self, input: CandidateBoundaryAuditInput<'_>) {
        self.record_boundary_audit(PrivilegeBoundaryAuditInput {
            stage: input.stage,
            request_kind: input.request_kind,
            operation: PrivilegedOperation::ApplyAction,
            policy_intent: Some(input.intent),
            caller_role: PrivilegeProcessRole::PrivilegedWorker,
            transport: PrivilegeTransport::LocalCli,
            descriptor: Some(input.descriptor),
            success: input.success,
            reason_code: input.reason_code,
            detail: input.detail,
            affected_tasks: input.affected_tasks,
        });
    }

    fn audit_rollback_boundary(
        &self,
        stage: &'static str,
        descriptor: &ActionDescriptor,
        success: bool,
        reason_code: &str,
        detail: &str,
        affected_tasks: usize,
    ) {
        self.record_boundary_audit(PrivilegeBoundaryAuditInput {
            stage,
            request_kind: "rollback",
            operation: PrivilegedOperation::RollbackAction,
            policy_intent: Some(PolicyIntent::Rollback),
            caller_role: PrivilegeProcessRole::PrivilegedWorker,
            transport: PrivilegeTransport::LocalCli,
            descriptor: Some(descriptor),
            success,
            reason_code,
            detail,
            affected_tasks,
        });
    }

    fn record_boundary_audit(&self, input: PrivilegeBoundaryAuditInput<'_>) {
        if let Err(err) = self.audit_sink.record_boundary(input) {
            log::warn!(
                "privilege_boundary_audit_failed path={} err={err:#}",
                self.audit_sink.path().display()
            );
        }
    }
}

impl PrivilegedActionService for InProcessPrivilegedActionService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        let descriptor = request.plan.descriptor.clone();
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "request_received",
            request_kind: "dry_run",
            intent: PolicyIntent::DryRun,
            descriptor: &descriptor,
            success: true,
            reason_code: "request_received",
            detail: "privilege dry-run request received",
            affected_tasks: 0,
        });
        if let Err(err) = validate_candidate_plan_request(&request, PolicyIntent::DryRun) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                stage: "policy_validation",
                request_kind: "dry_run",
                intent: PolicyIntent::DryRun,
                descriptor: &descriptor,
                success: false,
                reason_code: &reason_code,
                detail: &detail,
                affected_tasks: 0,
            });
            return Err(err);
        }
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "policy_validation",
            request_kind: "dry_run",
            intent: PolicyIntent::DryRun,
            descriptor: &descriptor,
            success: true,
            reason_code: "policy_allowed",
            detail: "candidate plan passed dry-run validation",
            affected_tasks: 0,
        });
        let executor = executor_for_candidate_preview(request.plan.candidate)?;
        let executor_descriptor = executor.descriptor();
        debug_assert_eq!(executor_descriptor.action_kind, descriptor.action_kind);
        debug_assert_eq!(executor.safety_class(), descriptor.safety_class);
        let executor_label = format!(
            "candidate={} action_kind={}",
            executor.candidate_name(),
            executor.action_kind()
        );
        let result = executor.dry_run();
        match result {
            Ok(record) => {
                self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                    stage: "dry_run_completed",
                    request_kind: "dry_run",
                    intent: PolicyIntent::DryRun,
                    descriptor: &descriptor,
                    success: true,
                    reason_code: "dry_run_completed",
                    detail: &format!("privileged dry-run completed {executor_label}"),
                    affected_tasks: record.affected_tasks,
                });
                Ok(record)
            }
            Err(err) => {
                let detail = format!("{err:#}");
                let reason_code = stable_error_reason_code(&detail);
                self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                    stage: "dry_run_failed",
                    request_kind: "dry_run",
                    intent: PolicyIntent::DryRun,
                    descriptor: &descriptor,
                    success: false,
                    reason_code: &reason_code,
                    detail: &detail,
                    affected_tasks: 0,
                });
                Err(err)
            }
        }
    }

    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        let descriptor = request.plan.descriptor.clone();
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "request_received",
            request_kind: "apply",
            intent: PolicyIntent::Apply,
            descriptor: &descriptor,
            success: true,
            reason_code: "request_received",
            detail: "privilege apply request received",
            affected_tasks: 0,
        });
        if let Err(err) = validate_candidate_plan_request(&request, PolicyIntent::Apply) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                stage: "policy_validation",
                request_kind: "apply",
                intent: PolicyIntent::Apply,
                descriptor: &descriptor,
                success: false,
                reason_code: &reason_code,
                detail: &detail,
                affected_tasks: 0,
            });
            return Err(err);
        }
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "policy_validation",
            request_kind: "apply",
            intent: PolicyIntent::Apply,
            descriptor: &descriptor,
            success: true,
            reason_code: "policy_allowed",
            detail: "candidate plan passed apply validation",
            affected_tasks: 0,
        });
        if let Err(err) = revalidate_candidate_targets(&request.plan.candidate, &self.proc_root) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                stage: "target_revalidation",
                request_kind: "apply",
                intent: PolicyIntent::Apply,
                descriptor: &descriptor,
                success: false,
                reason_code: &reason_code,
                detail: &detail,
                affected_tasks: 0,
            });
            return Err(err);
        }
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "apply_started",
            request_kind: "apply",
            intent: PolicyIntent::Apply,
            descriptor: &descriptor,
            success: true,
            reason_code: "apply_started",
            detail: "privileged apply started",
            affected_tasks: 0,
        });
        let apply_candidate =
            try_promote_to_apply_candidate(request.plan.candidate, ApplyEligibility::approved())
                .map_err(|eligibility| anyhow::anyhow!(eligibility.denial_message()))?;
        let executor = executor_for_apply_candidate(apply_candidate)?;
        let executor_descriptor = executor.descriptor();
        debug_assert_eq!(executor_descriptor.action_kind, descriptor.action_kind);
        debug_assert_eq!(executor.safety_class(), descriptor.safety_class);
        let executor_label = format!(
            "candidate={} action_kind={}",
            executor.candidate_name(),
            executor.action_kind()
        );
        let run_policy = ActionRunPolicy {
            policy: request.policy,
            context: request.context,
            max_affected_tasks: None,
            max_total_duration: None,
            dry_run: false,
        };
        let result = match executor.apply_with_audit(run_policy) {
            Ok(result) => result,
            Err(err) => {
                let detail = format!("{err:#}");
                let reason_code = stable_error_reason_code(&detail);
                self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                    stage: "apply_failed",
                    request_kind: "apply",
                    intent: PolicyIntent::Apply,
                    descriptor: &descriptor,
                    success: false,
                    reason_code: &reason_code,
                    detail: &detail,
                    affected_tasks: 0,
                });
                return Err(err);
            }
        };
        let rollback = match result.rollback {
            Some(rollback) => rollback,
            None => {
                let detail = "privileged apply completed without rollback token";
                self.audit_candidate_boundary(CandidateBoundaryAuditInput {
                    stage: "apply_failed",
                    request_kind: "apply",
                    intent: PolicyIntent::Apply,
                    descriptor: &descriptor,
                    success: false,
                    reason_code: "privileged_apply_missing_rollback",
                    detail,
                    affected_tasks: result.state.affected_tasks,
                });
                return Err(PrivilegedWorkerError::MissingRollbackToken.into());
            }
        };
        self.audit_candidate_boundary(CandidateBoundaryAuditInput {
            stage: "apply_completed",
            request_kind: "apply",
            intent: PolicyIntent::Apply,
            descriptor: &descriptor,
            success: true,
            reason_code: "apply_completed",
            detail: &format!("privileged apply completed {executor_label}"),
            affected_tasks: result.state.affected_tasks,
        });
        Ok(ApplyResult {
            state: result.state,
            rollback,
        })
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        let descriptor = request.candidate.descriptor();
        self.audit_rollback_boundary(
            "rollback_requested",
            &descriptor,
            true,
            "rollback_requested",
            "privilege rollback request received",
            request.token.affected_tasks(),
        );
        if let Err(err) = request.policy.check_action_with_context(
            PolicyIntent::Rollback,
            &descriptor,
            &request.context,
        ) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_rollback_boundary(
                "policy_validation",
                &descriptor,
                false,
                &reason_code,
                &detail,
                request.token.affected_tasks(),
            );
            return Err(err.into());
        }
        self.audit_rollback_boundary(
            "policy_validation",
            &descriptor,
            true,
            "policy_allowed",
            "rollback policy validation passed",
            request.token.affected_tasks(),
        );
        let affected_tasks = request.token.affected_tasks();
        let executor = executor_for_candidate_preview(request.candidate)?;
        if let Err(err) = executor.rollback(&request.token) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_rollback_boundary(
                "rollback_failed",
                &descriptor,
                false,
                &reason_code,
                &detail,
                affected_tasks,
            );
            return Err(err);
        }
        self.audit_rollback_boundary(
            "rollback_completed",
            &descriptor,
            true,
            "rollback_completed",
            "privileged rollback completed",
            affected_tasks,
        );
        Ok(RollbackResult { affected_tasks })
    }
}
