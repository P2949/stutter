use std::path::Path;

use anyhow::Context;

use super::{
    models::{UclampAction, UclampCurrentValues, UclampPolicy, UclampTargetSnapshot, UclampValues},
    system::{identity_warnings, read_target_snapshot_at, set_task_uclamp},
    validate::validate_policy_and_request,
};
use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, ApplyResult, RestoreIdentityStatus,
    RollbackToken, SafetyClass, TaskRestoreIdentity, TuningAction, UclampRestoreRecord,
    restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
    verify_task_identity,
};

impl UclampAction {
    pub fn preflight_with_policy(
        &self,
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    pub(crate) fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    pub(crate) fn dry_run_at(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
    ) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if requested_values_differ(self.values, snapshot.current) {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    fn verify_at(&self, proc_root: &Path, policy: &UclampPolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if requested_values_differ(self.values, snapshot.current) {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: !self.targets.is_empty() && pending_changes == 0,
            affected_tasks: self.targets.len(),
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    /// Original-value records for every target that really changes, each bound to the observed
    /// task identity. `require_identity` demands a start time so a recycled TID cannot match.
    fn plan_records(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
        require_identity: bool,
    ) -> anyhow::Result<Vec<(UclampRestoreRecord, UclampCurrentValues)>> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut plan = Vec::new();
        for (snapshot, _) in snapshots {
            if require_identity {
                anyhow::ensure!(
                    snapshot.starttime_ticks.is_some() && snapshot.process_pid.is_some(),
                    "uclamp target tid={} lacks a complete task identity",
                    snapshot.tid
                );
            }
            if !requested_values_differ(self.values, snapshot.current) {
                continue;
            }
            let identity = TaskRestoreIdentity::observed(
                snapshot.tid,
                snapshot.process_pid,
                snapshot.comm.clone(),
                snapshot.starttime_ticks,
                snapshot.exe.clone(),
            );
            let requested = UclampCurrentValues {
                sched_util_min: self.values.requested_min_or(snapshot.current),
                sched_util_max: self.values.requested_max_or(snapshot.current),
            };
            plan.push((
                UclampRestoreRecord::new(
                    identity,
                    snapshot.current.sched_util_min,
                    snapshot.current.sched_util_max,
                ),
                requested,
            ));
        }
        Ok(plan)
    }

    pub(crate) fn apply_prepared_at(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
        before_mutation: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> ApplyResult {
        let plan = self.plan_records(proc_root, policy, true)?;
        let token = uclamp_token(&plan);
        // The identity-complete original values are durable before the first write.
        before_mutation(&token)?;
        // Anything that moved between preparation and commit means the snapshot no longer
        // describes the host; abort with zero writes rather than restore a stale value.
        let current = self.plan_records(proc_root, policy, true)?;
        if !current
            .iter()
            .map(|(record, _)| record)
            .eq(plan.iter().map(|(record, _)| record))
        {
            return Err(anyhow::anyhow!(
                "uclamp target drifted between preparation and commit; nothing was written"
            )
            .into());
        }
        self.apply_plan(plan)
    }

    fn apply_plan(&self, plan: Vec<(UclampRestoreRecord, UclampCurrentValues)>) -> ApplyResult {
        crate::actions::transaction::ApplyTransaction::new().apply_planned_loop(
            plan,
            |(record, requested)| {
                set_task_uclamp(record.tid(), *requested).map_err(|e| {
                    e.context(format!("failed to set uclamp for tid={}", record.tid()))
                })
            },
            |records| RollbackToken::UclampRestore {
                records: records.into_iter().map(|(record, _)| record).collect(),
            },
        )
    }

    fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<(UclampTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.values)?;

        if self.targets.is_empty() {
            return Err(ActionBoundaryError::MissingExplicitTargets {
                action_kind: "uclamp",
            }
            .into());
        }

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            let snapshot = read_target_snapshot_at(proc_root, target)
                .with_context(|| format!("failed to preflight uclamp target tid={}", target.tid))?;
            let warnings = identity_warnings(target, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }
}

impl TuningAction for UclampAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "uclamp:set:min={}:max={}:targets:{}",
            optional_uclamp_value(self.values.sched_util_min),
            optional_uclamp_value(self.values.sched_util_max),
            self.targets.len()
        ))
    }

    fn describe(&self) -> String {
        let tids = self
            .targets
            .iter()
            .map(|target| target.tid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "set uclamp min={} max={} for task(s) [{}]",
            optional_uclamp_value(self.values.sched_util_min),
            optional_uclamp_value(self.values.sched_util_max),
            tids
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&UclampPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &UclampPolicy::default())
    }

    fn apply(&self) -> ApplyResult {
        let plan = self.plan_records(Path::new("/proc"), &UclampPolicy::default(), false)?;
        self.apply_plan(plan)
    }

    fn prepared_before_mutation(&self) -> bool {
        true
    }

    fn apply_with_prepared_rollback(
        &self,
        before_mutation: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> ApplyResult {
        self.apply_prepared_at(
            Path::new("/proc"),
            &UclampPolicy::default(),
            before_mutation,
        )
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &UclampPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_uclamp_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("uclamp-restore"),
            )
            .into());
        };

        let mut summary = RestoreSummary::default();
        let mut failures = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid.as_u32();
            match verify_task_identity(Path::new("/proc"), &identity) {
                RestoreIdentityStatus::SameTask => {}
                RestoreIdentityStatus::UnknownLegacy => {
                    log::warn!(
                        "uclamp rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::warn!("uclamp rollback skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "uclamp rollback skipped identity mismatch for tid={}: {}",
                        tid,
                        reason
                    );
                    continue;
                }
            }

            let requested = UclampCurrentValues {
                sched_util_min: record.original_util_min,
                sched_util_max: record.original_util_max,
            };

            match set_task_uclamp(tid, requested) {
                Ok(()) => summary.record_restored(),
                Err(err) => match classify_restore_write_error(Path::new("/proc"), tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::warn!("uclamp rollback skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "failed to restore uclamp min={} max={} for tid={}: {err:#}",
                            record.original_util_min, record.original_util_max, tid
                        ));
                    }
                },
            }
        }

        if summary.has_failures() {
            return Err(ActionBoundaryError::restore_failed(
                "uclamp",
                format!(
                    "failed to rollback uclamp after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                    summary.restored,
                    summary.skipped_missing,
                    summary.skipped_identity_mismatch,
                    summary.failed,
                    failures.join("; ")
                ),
            )
            .into());
        }

        Ok(())
    }
}
fn uclamp_token(plan: &[(UclampRestoreRecord, UclampCurrentValues)]) -> RollbackToken {
    RollbackToken::UclampRestore {
        records: plan.iter().map(|(record, _)| record.clone()).collect(),
    }
}

fn requested_values_differ(values: UclampValues, current: UclampCurrentValues) -> bool {
    values
        .sched_util_min
        .is_some_and(|requested| requested != current.sched_util_min)
        || values
            .sched_util_max
            .is_some_and(|requested| requested != current.sched_util_max)
}

fn optional_uclamp_value(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "keep".to_owned())
}
