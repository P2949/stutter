//! Stage implementations. Live-suite stages run through the privileged-runner backend and publish
//! a verified portable bundle; the benefit stages are in `benefit_stage`; integration and the
//! tier decision are in `decision`.
use std::fs;

use serde_json::{Value, json};

use super::{
    benefit_stage, decision,
    graph::Stage,
    runner::{BenefitHarness, StageContext, StageOutcome, StageRunner},
};
use crate::validation_campaign::{
    backend::{BackendError, SuiteBackend, SuiteInvocation},
    evidence::{Accept, PublishRequest, publish_bundle, record_failure},
    model::{BlockReason, FailureClass},
    state::write_atomic,
};

/// The production stage runner: live suites through `suites`, suspend/resume through `suspend`
/// (a direct run: this suite really suspends the machine), benefit through `harness`.
pub struct ProductionRunner {
    pub suites: Box<dyn SuiteBackend>,
    pub suspend: Box<dyn SuiteBackend>,
    pub harness: Box<dyn BenefitHarness>,
}

impl StageRunner for ProductionRunner {
    fn run(&self, context: &StageContext<'_>, stage: Stage, attempt: u32) -> StageOutcome {
        match stage {
            Stage::Preflight => preflight(context, attempt),
            Stage::Mechanical => run_suite_stage(
                context,
                stage,
                attempt,
                self.suites.as_ref(),
                context.operation.suites.mechanical.as_deref(),
            ),
            Stage::FaultMatrix => run_suite_stage(
                context,
                stage,
                attempt,
                self.suites.as_ref(),
                context.operation.suites.fault_matrix.as_deref(),
            ),
            Stage::SuspendResume => run_suite_stage(
                context,
                stage,
                attempt,
                self.suspend.as_ref(),
                context.operation.suites.suspend_resume.as_deref(),
            ),
            Stage::BenefitCalibration => {
                benefit_stage::calibrate(context, attempt, self.harness.as_ref())
            }
            Stage::BenefitHoldout => benefit_stage::holdout(context, attempt),
            Stage::KernelMatrix => StageOutcome::blocked(
                FailureClass::BlockedResource,
                "the kernel matrix for mutation operations is not implemented in this build",
            ),
            Stage::EvidenceIntegrate => decision::integrate(context, attempt),
            Stage::TierDecision => decision::tier(context, attempt),
        }
    }
}

/// Static `&'static str` suite names the backend takes.
fn suite_name(name: &str) -> Option<&'static str> {
    [
        "affinity",
        "recovery",
        "suspend-resume",
        "game-runnable",
        "observation",
    ]
    .into_iter()
    .find(|known| *known == name)
}

fn preflight(context: &StageContext<'_>, attempt: u32) -> StageOutcome {
    let dir = context.dir.attempt_dir(Stage::Preflight.as_str(), attempt);
    let checks = preflight_checks(context);
    let failed: Vec<&str> = checks
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(name, _, _)| *name)
        .collect();
    let body = json!({
        "schema_version": 1,
        "operation": context.state.operation,
        "objective": context.state.objective,
        "checks": checks
            .iter()
            .map(|(name, ok, detail)| json!({"check": name, "ok": ok, "detail": detail}))
            .collect::<Vec<_>>(),
    });
    let receipt = dir.join("preflight.json");
    if let Err(error) = write_atomic(
        &receipt,
        &serde_json::to_vec_pretty(&body).unwrap_or_default(),
    ) {
        return StageOutcome::failed(FailureClass::Retryable, "receipt", error.to_string());
    }
    if failed.is_empty() {
        StageOutcome::passed(
            Some(context.dir.relative(&dir)),
            Some(context.dir.relative(&receipt)),
        )
    } else {
        StageOutcome::Failed {
            class: FailureClass::SoftwareDefect,
            stage: "preflight".to_owned(),
            summary: format!("preflight checks failed: {}", failed.join(", ")),
            artifacts: Some(context.dir.relative(&dir)),
        }
    }
}

/// The operation is known to the product, its objective is mature enough to validate and not yet
/// Supported, and the registry does not already authorise it.
fn preflight_checks(context: &StageContext<'_>) -> Vec<(&'static str, bool, String)> {
    use stutter::api::autotune::{
        AUTONOMOUS_CONTRACTS, MutationOperationKind, ObjectiveKind, ObjectiveMaturity,
    };
    let operation = MutationOperationKind::ALL
        .into_iter()
        .find(|op| op.as_str() == context.state.operation);
    let objective = ObjectiveKind::ALL.into_iter().find(|kind| {
        serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            == Some(context.state.objective.clone())
    });
    let mut checks = vec![(
        "operation_known",
        operation.is_some_and(|op| !op.is_manual_only()),
        format!("operation {}", context.state.operation),
    )];
    let maturity = objective.map(|kind| kind.descriptor().maturity);
    checks.push((
        "objective_validating",
        maturity == Some(ObjectiveMaturity::Validating),
        format!(
            "objective {} maturity {maturity:?}",
            context.state.objective
        ),
    ));
    let registered = operation.is_some_and(|op| {
        AUTONOMOUS_CONTRACTS
            .iter()
            .any(|contract| contract.operation == op)
    });
    checks.push((
        "not_already_registered",
        !registered,
        "the autonomous support registry does not yet cover the operation".to_owned(),
    ));
    checks.push((
        "binaries_present",
        context.binaries.stutter.is_file() && context.binaries.xtask.is_file(),
        "pinned stutter and xtask binaries".to_owned(),
    ));
    checks
}

fn run_suite_stage(
    context: &StageContext<'_>,
    stage: Stage,
    attempt: u32,
    backend: &dyn SuiteBackend,
    configured: Option<&str>,
) -> StageOutcome {
    let Some(suite) = configured.and_then(suite_name) else {
        return StageOutcome::failed(
            FailureClass::SoftwareDefect,
            "configuration",
            format!("stage {} has no known suite configured", stage.as_str()),
        );
    };
    let extra_args: Vec<String> = if suite == "suspend-resume" {
        // Arm an RTC wake alarm so the machine resumes by itself: a campaign never waits for a person.
        vec![
            "--rtcwake-seconds".to_owned(),
            context.operation.suspend_wake_seconds.to_string(),
        ]
    } else {
        Vec::new()
    };
    let task = stage.as_str();
    let attempt_dir = context.dir.attempt_dir(task, attempt);
    let raw = attempt_dir.join("raw");
    let log = attempt_dir.join("logs/suite.log");
    let artifacts = Some(context.dir.relative(&attempt_dir));
    let invocation = SuiteInvocation {
        task,
        suite,
        attempt,
        seconds: 20,
        rounds: None,
        work_units: None,
        raw: &raw,
        log: &log,
        extra_args: &extra_args,
        stutter: &context.binaries.stutter,
        xtask: &context.binaries.xtask,
        cwd: context.repository,
        campaign_id: &context.state.campaign_id,
        manifest_sha256: &context.state.manifest_sha256,
    };
    match backend.execute(&invocation) {
        Ok(_) => {}
        Err(BackendError::Privilege(detail)) => {
            return StageOutcome::Blocked(BlockReason {
                class: FailureClass::BlockedPrivilege,
                detail,
            });
        }
        Err(BackendError::Failed(error)) => {
            return StageOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "launch".to_owned(),
                summary: format!("could not start the suite: {error}"),
                artifacts,
            };
        }
    }
    let result: Option<Value> = fs::read(raw.join("result.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let Some(result) = result else {
        return StageOutcome::Failed {
            class: FailureClass::Retryable,
            stage: "suite_result".to_owned(),
            summary: "the suite wrote no result.json".to_owned(),
            artifacts,
        };
    };
    if result["status"] == "BLOCKED" {
        return StageOutcome::Blocked(BlockReason {
            class: FailureClass::BlockedPrivilege,
            detail: result["error"]
                .as_str()
                .unwrap_or("suite blocked")
                .to_owned(),
        });
    }
    if !Accept::PassOnly.allows(&result) {
        return StageOutcome::Failed {
            class: FailureClass::Retryable,
            stage: "suite_result".to_owned(),
            summary: format!(
                "suite reported {}: {}",
                result["status"].as_str().unwrap_or("no status"),
                result["error"].as_str().unwrap_or("no error recorded")
            ),
            artifacts,
        };
    }
    match publish_bundle(&PublishRequest {
        dir: context.dir,
        repository: context.repository,
        task,
        attempt,
        raw: &raw,
        expected_commit: &context.state.evidence_base,
        allowed_kinds: &["controlled_real"],
        accept: Accept::PassOnly,
    }) {
        Ok(published) => {
            StageOutcome::passed(artifacts, Some(context.dir.relative(&published.receipt)))
        }
        Err(failure) => {
            let _ = record_failure(context.dir, task, attempt, &failure);
            let class = match failure.stage.as_str() {
                "fsync" | "publish" | "receipt" => FailureClass::Retryable,
                _ => FailureClass::SoftwareDefect,
            };
            StageOutcome::Failed {
                class,
                stage: failure.stage,
                summary: failure.summary,
                artifacts,
            }
        }
    }
}
