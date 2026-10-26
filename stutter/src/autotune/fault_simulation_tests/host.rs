//! A simulated host behind the real executor boundary: one resource per conflict group, durable
//! rollback tokens that name their restore file, and scripted faults.
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc};

use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        conflicts::ActionConflictGroup,
        controller_journal::{
            ControllerJournalState, write_controller_journal_applied_with_metadata,
            write_controller_journal_phase_with_metadata,
        },
        experiment::ExperimentId,
        live_experiment::{
            LiveExperiment, LiveExperimentActionExecutor, LiveExperimentManagerInput,
        },
        observation::{ActiveConfigSnapshot, AutotuneObservation},
        planning::candidate::CandidateAction,
        startup_recovery::{StartupRecoveryRollbackExecutor, StartupRecoveryRollbackSummary},
    },
};

/// One scripted failure, consumed by the next matching host operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::autotune) enum Fault {
    /// The apply fails before any write.
    ApplyRefused,
    /// The apply writes, then the worker dies before answering: the effect is live, the token
    /// is durable in the journal, and the caller sees an error.
    WorkerDiesAfterWrite,
    /// The next rollback of any token fails and leaves the effect in place.
    RollbackFails,
    /// The privileged worker is unreachable for every operation until restarted.
    WorkerDown,
}

struct PreparedWrite {
    group: ActionConflictGroup,
    resource: String,
    written: i64,
    path: PathBuf,
}

/// What an applied token did to the host, enough to verify an identity-checked restore.
#[derive(Clone, Debug)]
pub(super) struct HostEffect {
    pub(super) group: ActionConflictGroup,
    pub(super) resource: String,
    pub(super) original: i64,
    pub(super) written: i64,
}

#[derive(Debug, Default)]
pub(in crate::autotune) struct SimHost {
    resources: BTreeMap<String, i64>,
    /// Effects that are live on the host and not yet restored, by token path.
    effects: BTreeMap<PathBuf, HostEffect>,
    faults: Vec<Fault>,
    worker_down: bool,
    next_value: i64,
    /// Every host mutation, for the audit-completeness check.
    pub(super) mutations: usize,
    /// The (operation, objective, situation) of every apply that reached the host.
    pub(super) applied_tuples: Vec<String>,
}

pub(super) type SharedHost = Rc<RefCell<SimHost>>;

impl SimHost {
    pub(super) fn shared() -> SharedHost {
        Rc::new(RefCell::new(Self::default()))
    }

    pub(super) fn inject(&mut self, fault: Fault) {
        if fault == Fault::WorkerDown {
            self.worker_down = true;
        } else {
            self.faults.push(fault);
        }
    }

    pub(super) fn restart_worker(&mut self) {
        self.worker_down = false;
    }

    fn take(&mut self, fault: Fault) -> bool {
        match self.faults.iter().position(|f| *f == fault) {
            Some(index) => {
                self.faults.remove(index);
                true
            }
            None => false,
        }
    }

    pub(super) fn value(&self, resource: &str) -> i64 {
        self.resources.get(resource).copied().unwrap_or(0)
    }

    /// Something other than the controller changes a resource the controller also touched.
    pub(super) fn external_write(&mut self, group: ActionConflictGroup) {
        self.next_value += 1;
        self.resources
            .insert(format!("{group:?}"), 1_000 + self.next_value);
    }

    pub(super) fn live_effects(&self) -> impl Iterator<Item = (&PathBuf, &HostEffect)> {
        self.effects.iter()
    }

    pub(super) fn reboot(&mut self) {
        // Runtime resources do not survive a boot; the tokens that named them are now stale.
        self.resources.clear();
        self.effects.clear();
        self.faults.clear();
        self.worker_down = false;
    }

    /// Phase one of a prepared apply: choose the durable token. Nothing is written.
    fn prepare(
        &mut self,
        group: ActionConflictGroup,
        resource: String,
    ) -> anyhow::Result<PreparedWrite> {
        anyhow::ensure!(
            !self.worker_down,
            "privileged_worker_unreachable: worker is down"
        );
        anyhow::ensure!(
            !self.take(Fault::ApplyRefused),
            "apply refused before any write"
        );
        self.next_value += 1;
        let written = self.next_value;
        Ok(PreparedWrite {
            group,
            path: PathBuf::from(format!("/tmp/stutter-sim/{resource}-{written}.json")),
            resource,
            written,
        })
    }

    /// Phase two: the first write, only after the caller made the token durable.
    fn commit(&mut self, prepared: &PreparedWrite, tuple: String) {
        let original = self.value(&prepared.resource);
        self.resources
            .insert(prepared.resource.clone(), prepared.written);
        self.mutations += 1;
        self.applied_tuples.push(tuple);
        self.effects.insert(
            prepared.path.clone(),
            HostEffect {
                group: prepared.group,
                resource: prepared.resource.clone(),
                original,
                written: prepared.written,
            },
        );
    }

    /// Identity-checked restore: it never overwrites a value the controller did not write.
    pub(super) fn restore(&mut self, path: &PathBuf) -> anyhow::Result<usize> {
        anyhow::ensure!(
            !self.worker_down,
            "privileged_worker_unreachable: worker is down"
        );
        if self.take(Fault::RollbackFails) {
            anyhow::bail!("rollback failed: scripted fault");
        }
        let Some(effect) = self.effects.get(path).cloned() else {
            // Already restored, or retired by a reboot: nothing to undo and nothing to clobber.
            return Ok(0);
        };
        if self.value(&effect.resource) != effect.written {
            // An external writer owns the value now. Retire the token without overwriting it.
            self.effects.remove(path);
            return Ok(0);
        }
        self.resources
            .insert(effect.resource.clone(), effect.original);
        self.effects.remove(path);
        self.mutations += 1;
        Ok(1)
    }
}

pub(super) fn token_for(path: PathBuf) -> RollbackToken {
    RollbackToken::CpuAffinityRestoreFile {
        path,
        affected_tasks: 1,
    }
}

pub(super) fn token_path(token: &RollbackToken) -> Option<PathBuf> {
    token.restore_path().cloned()
}

/// The live-experiment executor over the simulated host. It journals exactly as the real
/// executors do: Applying, then Applied(token), both durable before the call returns.
pub(super) struct SimExecutor {
    pub(super) host: SharedHost,
}

impl LiveExperimentActionExecutor for SimExecutor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        experiment_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        let journal = input
            .controller_journal_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the simulation always has a journal"))?;
        let metadata = |verify: &str| {
            crate::autotune::controller_journal::ControllerJournalActionMetadata::default()
                .with_candidate(candidate.profile_name().to_owned())
                .with_mode(input.mode)
                .with_safety_class(candidate.safety_class())
                .with_verify_result(verify)
        };
        let experiment = ExperimentId::try_new(experiment_id)?;
        let resource = if candidate.is_mutation_provider() {
            format!("{:?}", candidate.conflict_group())
        } else {
            candidate.action_id().as_str().to_owned()
        };
        let prepared = self
            .host
            .borrow_mut()
            .prepare(candidate.conflict_group(), resource)?;
        let token = token_for(prepared.path.clone());
        // Prepared-before-mutation: Applying(token) is durable before the first write.
        write_controller_journal_phase_with_metadata(
            &journal,
            ControllerJournalState::Applying,
            experiment.clone(),
            candidate.action_id(),
            Some(token.clone()),
            metadata("rollback_prepared_before_mutation"),
        )?;
        let tuple = if candidate.is_mutation_provider() {
            format!(
                "{:?}/{:?}/{:?}",
                candidate.operation_kind(),
                candidate.objective(),
                observation.primary_situation
            )
        } else {
            "test-only-fake".to_owned()
        };
        self.host.borrow_mut().commit(&prepared, tuple);
        // Low-risk executors own the Applied record; the medium-risk manager writes its own.
        if candidate.safety_class() <= SafetyClass::ReversibleLowRisk {
            write_controller_journal_applied_with_metadata(
                &journal,
                experiment,
                candidate.action_id(),
                token.clone(),
                metadata("sim_verified"),
            )?;
        }
        if self.host.borrow_mut().take(Fault::WorkerDiesAfterWrite) {
            self.host.borrow_mut().inject(Fault::WorkerDown);
            anyhow::bail!("privileged_worker_died: connection lost after the write");
        }
        Ok(token)
    }

    fn rollback_candidate(
        &mut self,
        _input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        _observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>> {
        let path = token_path(&experiment.rollback)
            .ok_or_else(|| anyhow::anyhow!("the simulation only issues path tokens"))?;
        self.host.borrow_mut().restore(&path)?;
        Ok(None)
    }
}

/// Startup recovery after a controller restart restores through the (possibly restarted) worker.
pub(super) struct SimRecoveryExecutor {
    pub(super) host: SharedHost,
}

impl StartupRecoveryRollbackExecutor for SimRecoveryExecutor {
    fn rollback(
        &mut self,
        token: &RollbackToken,
    ) -> anyhow::Result<StartupRecoveryRollbackSummary> {
        let path = token_path(token).ok_or_else(|| anyhow::anyhow!("not a simulation token"))?;
        let restored = self.host.borrow_mut().restore(&path)?;
        Ok(StartupRecoveryRollbackSummary {
            affected_tasks: restored,
            message: format!("sim restore restored={restored}"),
        })
    }
}
