//! Root-owned service policy and registry-derived operation envelope.
use std::os::unix::fs::MetadataExt;

use super::*;

/// Resolve the explicitly monitored desktop account from the local passwd database. No program is
/// run: a systemd unit has no dependable `PATH`, and an unknown account is refused (fail closed).
/// A service instance must never select root or interpret an account as command options.
pub(crate) fn controller_uid(user: &str) -> anyhow::Result<u32> {
    anyhow::ensure!(
        !user.is_empty()
            && user
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
        "invalid controller user"
    );
    let uid = uid_in_passwd(&fs::read_to_string("/etc/passwd")?, user)
        .with_context(|| format!("unknown controller user {user:?}"))?;
    anyhow::ensure!(
        uid != 0,
        "bounded controller must run as a non-root desktop user"
    );
    Ok(uid)
}

fn uid_in_passwd(passwd: &str, user: &str) -> Option<u32> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != user {
            return None;
        }
        fields.nth(1)?.parse().ok()
    })
}

pub fn run_privileged_service_worker(
    socket_path: &Path,
    user: &str,
    recover_only: bool,
) -> anyhow::Result<()> {
    let uid = controller_uid(user)?;
    let config_path = std::env::var_os("STUTTER_SERVICE_CONFIG")
        .map(PathBuf::from)
        .context("bounded worker requires a root-owned STUTTER_SERVICE_CONFIG")?;
    for path in config_path.ancestors() {
        let metadata = fs::metadata(path)?;
        anyhow::ensure!(
            metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
            "bounded worker configuration and ancestors must be root-owned and not group/world writable"
        );
    }
    let user_config = crate::config_file::load_user_config_at(&config_path)?;
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        crate::daemon_policy::DaemonMode::ApplyMediumRisk,
        crate::daemon_policy::ActionSource::AutotuneRuntime,
        None,
        None,
    );
    crate::config_file::apply_daemon_user_config_overrides(&mut config, Some(&user_config));
    config.autotune.allow_medium_risk_apply = true;
    config.safety.allow_high_risk = false;
    config.safety.allow_persistent_effects = false;
    let policy =
        crate::daemon_policy::build_daemon_policy(crate::daemon_policy::DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
    let service = super::service_tokens::ServiceActionService::new(
        &std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("service worker requires its private state HOME")?,
        uid,
        user_config
            .autotune
            .as_ref()
            .and_then(|config| config.validation_workload_cgroup.clone()),
    );
    service.recover_issued(&policy)?;
    if recover_only {
        return Ok(());
    }
    super::worker::run_worker(socket_path, &service, Some((uid, policy)))
}

/// The service never accepts a caller-selected mutation ceiling. Its operation envelope is
/// derived from the product registry; live workload/health gates remain in the controller.
pub(super) fn validate_request(request: &PrivilegedWorkerRequest) -> anyhow::Result<()> {
    let plan = match request {
        PrivilegedWorkerRequest::ApplyPrepared { plan, .. }
        | PrivilegedWorkerRequest::DryRun { plan, .. }
        | PrivilegedWorkerRequest::Rollback { plan, .. } => plan,
        PrivilegedWorkerRequest::Apply { .. } => {
            return Err(PrivilegedWorkerError::BoundedServiceRequiresPrepared.into());
        }
        PrivilegedWorkerRequest::Shutdown | PrivilegedWorkerRequest::SettleIssuedTokens => {
            return Ok(());
        }
        PrivilegedWorkerRequest::CommitPrepared { .. } => {
            return Err(PrivilegedWorkerError::CommitRejected {
                reason: "commit without prepare".to_owned(),
            }
            .into());
        }
    };
    let candidate = plan.clone().into_plan_request()?.candidate;
    if !matches!(request, PrivilegedWorkerRequest::Rollback { .. })
        && let crate::autotune::planning::candidate::CandidateAction::CpuAffinityProfile { plan } =
            &candidate
    {
        anyhow::ensure!(
            plan.tree_starttime_ticks.is_some_and(|ticks| ticks > 0),
            PrivilegedWorkerError::CandidatePlanMetadataMismatch
        );
    }
    if !matches!(request, PrivilegedWorkerRequest::Rollback { .. })
        && let crate::autotune::planning::candidate::CandidateAction::CgroupPlacement { plan } =
            &candidate
    {
        anyhow::ensure!(
            plan.target_root_pid.is_some()
                && plan
                    .target_root_starttime_ticks
                    .is_some_and(|ticks| ticks > 0),
            PrivilegedWorkerError::CandidatePlanMetadataMismatch
        );
    }
    let descriptor = candidate.descriptor();
    anyhow::ensure!(
        candidate.operation_kind() == plan.operation
            && candidate.objective() == plan.objective
            && serde_json::to_value(&descriptor)? == serde_json::to_value(&plan.descriptor)?,
        "bounded_service_plan_mismatch"
    );
    anyhow::ensure!(
        descriptor.safety_class <= crate::actions::SafetyClass::ReversibleMediumRisk,
        "bounded_service_high_risk_denied"
    );
    let registered = crate::autotune::autonomous_support::AUTONOMOUS_CONTRACTS
        .iter()
        .any(|contract| {
            contract.operation == candidate.operation_kind()
                && contract.objective == candidate.objective()
                && contract.effect_scope == descriptor.effect_scope
        });
    anyhow::ensure!(
        registered
            && candidate
                .operation_kind()
                .prepared_transaction_implemented(),
        "bounded_service_operation_not_supported"
    );
    anyhow::ensure!(
        candidate.objective().descriptor().maturity
            == crate::autotune::objective::ObjectiveMaturity::Supported,
        "bounded_service_objective_not_supported"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire(objective: crate::autotune::objective::ObjectiveKind) -> PrivilegedWorkerCandidatePlan {
        let candidate =
            crate::autotune::planning::candidate::CandidateAction::cpu_affinity_profile_for(
                crate::profiles::Profile {
                    name: "bounded-wire".into(),
                    rules: vec![],
                },
                std::process::id(),
                objective,
            );
        PrivilegedWorkerCandidatePlan::from_plan_request(&CandidatePlanRequest::from_candidate(
            candidate,
            crate::audit::unix_nanos_now(),
        ))
    }
    #[test]
    fn bounded_worker_derives_its_operation_envelope_from_registry_and_requires_prepare() {
        let plan = wire(crate::autotune::objective::ObjectiveKind::GameRunnableLatency);
        let request = PrivilegedWorkerRequest::ApplyPrepared {
            plan: plan.clone(),
            policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                crate::daemon_policy::ActionSource::Test,
            ),
            context: Default::default(),
            max_plan_age_nanos: 1_000_000_000,
        };
        let request: PrivilegedWorkerRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        validate_request(&request).unwrap();
        let mut unsupported = request.clone();
        if let PrivilegedWorkerRequest::ApplyPrepared { plan, .. } = &mut unsupported {
            *plan = wire(crate::autotune::objective::ObjectiveKind::StutterScore);
        }
        assert!(validate_request(&unsupported).is_err());
        let unprepared = PrivilegedWorkerRequest::Apply {
            plan,
            policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                crate::daemon_policy::ActionSource::Test,
            ),
            context: Default::default(),
            max_plan_age_nanos: 1_000_000_000,
        };
        assert!(
            format!("{:#}", validate_request(&unprepared).unwrap_err())
                .contains("bounded_service_requires_prepared_transaction")
        );
        assert!(controller_uid("root").is_err());
        assert!(controller_uid("--help").is_err());
    }

    #[test]
    fn the_controller_uid_comes_from_the_passwd_database_without_a_program() {
        let passwd = "root:x:0:0::/root:/bin/sh\nalice:x:1000:100::/home/alice:/bin/sh\nbad:x:nan:1::/:/bin/sh\n";
        assert_eq!(uid_in_passwd(passwd, "alice"), Some(1000));
        assert_eq!(uid_in_passwd(passwd, "alic"), None);
        assert_eq!(uid_in_passwd(passwd, "bad"), None);
    }
}
