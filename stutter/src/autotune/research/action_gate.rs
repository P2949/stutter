//! The per-candidate gate that replaces the blanket "research owns the slot, so no autonomous
//! action" suppression (PLAN section 7). While a CP-LONG action child runs, the normal controller
//! still plans and applies through its own journalled, two-phase, token-bearing experiment
//! lifecycle; this gate only decides whether a given candidate may proceed (an authorized game
//! action on the frozen game process, one at a time) and observes what the experiment manager
//! reports back (an experiment started, the action kept, or rolled back, and whether the worker
//! still holds a token), so the `ActionChildScope` can settle only with a proven restoration. It
//! applies nothing itself and is no second optimizer.

use std::sync::{Arc, Mutex};

use stutter_core::ids::{ExperimentId, Pid};

use super::{
    ResearchAction,
    action_child::{ActionChildScope, ActionGrant, ActionRequest, ChildTarget, Decision},
};

/// One applied or kept action the experiment manager holds: which experiment (the controller's
/// transaction), which supported action, on which process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub experiment_id: ExperimentId,
    pub action: Option<ResearchAction>,
    pub pid: Option<Pid>,
}

/// What the experiment manager and worker report after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateView {
    /// Every action the manager holds (active or kept). Empty when the caller cannot attribute them;
    /// the aggregate counts below still apply.
    pub holdings: Vec<Holding>,
    /// An experiment is applied and being measured.
    pub experiment_active: bool,
    /// Actions the controller kept (they stay applied until released).
    pub kept_actions: usize,
    /// Worker transactions still outstanding; `None` when the worker cannot say.
    pub outstanding_tokens: Option<u32>,
}

enum State {
    Idle,
    /// A candidate was permitted; the apply happens within the same event.
    Requested(ActionGrant, ResearchAction),
    /// Applied and under measurement.
    Running(ResearchAction),
    /// Kept by the controller; stays applied until released.
    Kept(ResearchAction),
    /// Undone by the controller; waiting for the worker to hold no token.
    Restoring(ResearchAction),
}

/// Reads the worker's outstanding transaction count (`None` when it cannot say).
pub type TokenProbe = Box<dyn Fn() -> Option<u32> + Send>;

/// The kernel-visible placement of a process that the two supported actions change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KernelState {
    /// The leader's allowed-CPU list.
    pub cpus_allowed: String,
    pub cgroup: String,
    /// Every thread's allowed-CPU list, by thread id (the actions change the whole game tree).
    pub tasks: std::collections::BTreeMap<u32, String>,
    /// The process's start time and owner, read fresh with the rest (`None` from a fake probe).
    pub identity: Option<(u64, u32)>,
}

impl KernelState {
    /// This state is the original one: the same cgroup and leader placement, the same process
    /// (start time and owner, when the original recorded them), and every thread the original saw
    /// that still exists back on its own CPUs. A thread that has since exited is not a difference.
    pub fn restored_to(&self, original: &KernelState) -> bool {
        self.cgroup == original.cgroup
            && self.cpus_allowed == original.cpus_allowed
            && (original.identity.is_none() || self.identity == original.identity)
            && original
                .tasks
                .iter()
                .all(|(tid, cpus)| self.tasks.get(tid).is_none_or(|now| now == cpus))
    }
}

/// Reads a process's kernel state independently of the worker (`None`: cannot say).
pub type StateProbe = Box<dyn Fn(u32) -> Option<KernelState> + Send>;

/// The state from `/proc`: the allowed-CPU list and the cgroup membership.
pub fn procfs_state(pid: u32) -> Option<KernelState> {
    // The same process (pid, start time, owner) must hold before and after the capture: a pid that was
    // recycled while the threads were being read would otherwise yield a state of two processes.
    let before = process_identity(pid);
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let cpus = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?
        .trim()
        .to_owned();
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    // Every thread of the process, bounded. A thread that exited while reading is gone; one that is
    // still listed but cannot be read, an enumeration that fails part-way, or a process with more
    // threads than the bound leaves the whole state unknown rather than a partial answer.
    let listed = task_entries(
        std::fs::read_dir(format!("/proc/{pid}/task"))
            .ok()?
            .map(|entry| entry.map(|entry| (entry.file_name(), entry.path()))),
        MAX_TASKS,
    )?;
    let tasks = parse_tasks(
        listed
            .into_iter()
            .map(|(tid, path)| (tid, std::fs::read_to_string(path.join("status")))),
    )?;
    let identity = steady_identity(before, process_identity(pid));
    Some(KernelState {
        cpus_allowed: cpus,
        cgroup: cgroup.trim().to_owned(),
        tasks,
        identity: Some(identity?),
    })
}

/// Whether a read error says the thread no longer exists: its `/proc` entry is gone (`NotFound`) or the
/// kernel reports the task has exited while its file was being read (`ESRCH`). Either way it is an
/// exited thread, not an unreadable live one.
fn thread_is_gone(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

/// The most threads one process's state is read for; more is unknown, never a truncated answer.
const MAX_TASKS: usize = 4096;

/// The threads of a process from its `task` directory listing, as (thread id, directory). An entry
/// that vanished while listing is gone; any other listing error, a name that is not a thread id, or
/// more than `max` entries makes the whole answer `None`.
fn task_entries(
    entries: impl Iterator<Item = std::io::Result<(std::ffi::OsString, std::path::PathBuf)>>,
    max: usize,
) -> Option<Vec<(u32, std::path::PathBuf)>> {
    let mut listed = Vec::new();
    for entry in entries {
        let (name, path) = match entry {
            Ok(entry) => entry,
            Err(error) if thread_is_gone(&error) => continue,
            Err(_) => return None,
        };
        if listed.len() >= max {
            return None;
        }
        listed.push((name.to_str()?.parse::<u32>().ok()?, path));
    }
    Some(listed)
}

/// The identity of a process observed to be the same before and after a capture, `None` when either
/// observation failed or they differ.
fn steady_identity(before: Option<ChildTarget>, after: Option<ChildTarget>) -> Option<(u64, u32)> {
    let (before, after) = (before?, after?);
    (before == after).then_some((before.start_ticks, before.uid))
}

/// The allowed-CPU list of every thread from its status text. A thread whose status vanished (it
/// exited) is skipped; one that fails to read for any other reason makes the whole answer `None`:
/// an unreadable live thread must not look like an exited one.
fn parse_tasks(
    items: impl Iterator<Item = (u32, std::io::Result<String>)>,
) -> Option<std::collections::BTreeMap<u32, String>> {
    let mut tasks = std::collections::BTreeMap::new();
    for (tid, text) in items {
        let text = match text {
            Ok(text) => text,
            Err(error) if thread_is_gone(&error) => continue,
            Err(_) => return None,
        };
        let list = text
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?;
        tasks.insert(tid, list.trim().to_owned());
    }
    Some(tasks)
}

/// One applied action's receipt: the frozen target, what the kernel showed before the apply and what
/// it shows after the restoration was accepted (equal, or the restoration was not accepted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionReceipt {
    /// The controller's experiment (transaction) identity, when the manager reported one.
    pub transaction: Option<ExperimentId>,
    pub action: ResearchAction,
    pub target: ChildTarget,
    pub original: KernelState,
    pub restored: KernelState,
}

pub struct ActionGate {
    /// `None` until an action child arms the gate on the sighted game process; unarmed, nothing is
    /// ever permitted.
    scope: Option<ActionChildScope>,
    state: State,
    tokens: TokenProbe,
    /// Independent kernel-state reader; without one no restoration can be proven against the kernel.
    state_probe: Option<StateProbe>,
    /// What the target's kernel state was when the action was permitted.
    baseline: Option<(ChildTarget, KernelState)>,
    receipts: Vec<ActionReceipt>,
    /// The controller experiment the permitted action became (captured when it was first seen applied).
    transaction: Option<ExperimentId>,
    /// The frozen process of the permitted action.
    target: Option<ChildTarget>,
    /// Something other than the permitted action on the frozen process was held while the gate was
    /// busy: the evidence cannot be attributed to the permitted action (sticky).
    foreign: bool,
    /// In a within-source study only attempts treated On may have an action applied; every other
    /// study leaves this true.
    attempt_on: bool,
    /// What earlier attempts' scopes settled in this child (each On attempt is its own process and its
    /// own scope; their outcomes accumulate here).
    carried: Vec<(ResearchAction, Decision)>,
}

impl std::fmt::Debug for ActionGate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionGate")
            .field("busy", &self.is_busy())
            .field("armed", &self.is_armed())
            .finish_non_exhaustive()
    }
}

/// Shared between the funnel (which arms it and reads its evidence) and the runtime (which asks
/// it for permission and reports what happened).
pub type SharedActionGate = Arc<Mutex<ActionGate>>;

/// What the gate saw, for the study's stage evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateEvidence {
    /// Actions that were applied, decided and whose restoration was proven.
    pub settled: Vec<(ResearchAction, Decision)>,
    /// Nothing applied or pending: every granted action was restored (or never applied).
    pub clean: bool,
    /// Each proven restoration, with the kernel state before the apply and after the restore.
    pub receipts: Vec<ActionReceipt>,
    /// Every settled action was compared against the target's own kernel state (not just a zero
    /// worker-token count).
    pub kernel_state_proven: bool,
}

impl GateEvidence {
    /// The action receipts as the durable JSON recorded beside the action study.
    pub fn receipts_json(&self) -> serde_json::Value {
        let state = |state: &KernelState| {
            serde_json::json!({
                "cpus_allowed": state.cpus_allowed,
                "cgroup": state.cgroup,
                "tasks": state.tasks,
                "identity": state.identity,
            })
        };
        serde_json::Value::Array(
            self.receipts
                .iter()
                .map(|receipt| {
                    serde_json::json!({
                        "transaction": receipt.transaction.as_ref().map(ExperimentId::as_str),
                        "action": format!("{:?}", receipt.action),
                        "pid": u32::from(receipt.target.pid),
                        "start_ticks": receipt.target.start_ticks,
                        "uid": receipt.target.uid,
                        "original": state(&receipt.original),
                        "restored": state(&receipt.restored),
                    })
                })
                .collect(),
        )
    }
}

impl ActionGate {
    pub fn new(scope: ActionChildScope, tokens: TokenProbe) -> Self {
        Self {
            scope: Some(scope),
            state: State::Idle,
            tokens,
            state_probe: None,
            baseline: None,
            receipts: Vec::new(),
            transaction: None,
            target: None,
            foreign: false,
            attempt_on: true,
            carried: Vec::new(),
        }
    }

    /// Prove restorations against the target's own kernel state, read independently of the worker.
    pub fn with_state_probe(mut self, probe: StateProbe) -> Self {
        self.state_probe = Some(probe);
        self
    }

    /// A gate that permits nothing until it is armed.
    pub fn unarmed(tokens: TokenProbe) -> Self {
        Self {
            scope: None,
            state: State::Idle,
            tokens,
            state_probe: None,
            baseline: None,
            receipts: Vec::new(),
            transaction: None,
            target: None,
            foreign: false,
            attempt_on: true,
            carried: Vec::new(),
        }
    }

    /// Arm the gate for one action child. Refused while a previous child's action is outstanding.
    pub fn arm(&mut self, scope: ActionChildScope) -> bool {
        if self.is_busy() {
            return false;
        }
        self.receipts.clear();
        self.carried.clear();
        self.foreign = false;
        self.attempt_on = true;
        self.scope = Some(scope);
        true
    }

    /// The next attempt is a new game process: move the gate onto it. Refused while an action is
    /// outstanding or when unarmed; what earlier attempts settled is kept, and the evidence (receipts,
    /// attribution) carries on.
    pub fn rebind(&mut self, target: ChildTarget) -> bool {
        if self.is_busy() {
            return false;
        }
        let Some(next) = self
            .scope
            .as_ref()
            .and_then(|scope| scope.retargeted(target))
        else {
            return false;
        };
        if let Some(previous) = self.scope.as_ref() {
            self.carried.extend(previous.settled().iter().copied());
        }
        self.scope = Some(next);
        true
    }

    /// Whether the attempt now running is one the action may be applied in (a treatment study's
    /// attempts treated Off are not).
    pub fn set_attempt_on(&mut self, on: bool) {
        self.attempt_on = on;
    }

    pub fn is_armed(&self) -> bool {
        self.scope.is_some()
    }

    /// The action child ended: the gate is disarmed and what it saw is returned. A child that left an
    /// action outstanding is not clean (and the gate stays busy until it is restored).
    pub fn disarm(&mut self) -> GateEvidence {
        let evidence = self.evidence();
        if evidence.clean {
            self.scope = None;
        }
        evidence
    }

    /// Report the experiment manager's state after an event; the worker's token count is read here.
    pub fn observe_runtime(&mut self, experiment_active: bool, kept_actions: usize) {
        self.observe_holdings(experiment_active, kept_actions, Vec::new());
    }

    /// As `observe_runtime`, with every action the manager holds so each can be attributed.
    pub fn observe_holdings(
        &mut self,
        experiment_active: bool,
        kept_actions: usize,
        holdings: Vec<Holding>,
    ) {
        let outstanding_tokens = (self.tokens)();
        self.observe(GateView {
            holdings,
            experiment_active,
            kept_actions,
            outstanding_tokens,
        });
    }

    fn with_scope<R>(&mut self, f: impl FnOnce(&mut ActionChildScope) -> R) -> Option<R> {
        self.scope.as_mut().map(f)
    }

    /// Whether an action is outstanding (granted, applied, kept or being restored).
    pub fn is_busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// Permission for the normal controller to apply a candidate. Only an authorized game action
    /// on the frozen process, inside the window, one at a time, is permitted; everything else is
    /// refused exactly as the blanket suppression did.
    pub fn permit(
        &mut self,
        action: Option<ResearchAction>,
        target: ChildTarget,
        now_unix: u64,
    ) -> bool {
        let Some(action) = action else { return false };
        if !self.attempt_on || !matches!(self.state, State::Idle) {
            return false;
        }
        // With a state probe the original state is captured first; a target whose state cannot be read
        // cannot have its restoration proven, so nothing is applied to it.
        let baseline = match &self.state_probe {
            Some(probe) => match probe(u32::from(target.pid)) {
                Some(state) => Some((target, state)),
                None => return false,
            },
            None => None,
        };
        let Some(scope) = self.scope.as_mut() else {
            return false;
        };
        match scope.authorize(ActionRequest {
            action,
            target,
            now_unix,
        }) {
            Ok(grant) => {
                self.baseline = baseline;
                self.target = Some(target);
                self.transaction = None;
                self.state = State::Requested(grant, action);
                true
            }
            Err(_) => false,
        }
    }

    /// Report what the experiment manager and worker show after an event.
    pub fn observe(&mut self, view: GateView) {
        if !self.attributable(&view) {
            self.foreign = true;
            return;
        }
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match state {
            State::Idle => State::Idle,
            State::Requested(grant, action) => {
                if view.experiment_active || view.kept_actions > 0 {
                    self.transaction = view.holdings.first().map(|held| held.experiment_id.clone());
                    // The normal path verified its own apply before starting the experiment.
                    self.with_scope(|scope| scope.applied(grant, true));
                    State::Running(action)
                } else {
                    // The controller declined or failed to apply: nothing changed.
                    self.with_scope(|scope| scope.not_applied(grant));
                    State::Idle
                }
            }
            State::Running(action) => {
                if view.experiment_active {
                    State::Running(action)
                } else if view.kept_actions > 0 {
                    self.with_scope(|scope| scope.decide(Decision::Keep));
                    State::Kept(action)
                } else {
                    self.with_scope(|scope| scope.decide(Decision::Revert));
                    self.restore_if_proven(action, &view)
                }
            }
            State::Kept(action) => {
                if view.kept_actions == 0 {
                    // Released by the controller (its own rollback path verified the restore).
                    self.restore_if_proven(action, &view)
                } else {
                    State::Kept(action)
                }
            }
            State::Restoring(action) => self.restore_if_proven(action, &view),
        };
    }

    /// Every action the manager holds belongs to the permitted action on the frozen process, and to
    /// one controller experiment. Idle gates attribute nothing, so they accept any view.
    fn attributable(&self, view: &GateView) -> bool {
        // An experiment or kept action reported with nothing identifying it cannot be attributed.
        let held = view.experiment_active || view.kept_actions > 0;
        if held
            && view.holdings.is_empty()
            && matches!(
                self.state,
                State::Requested(..) | State::Running(_) | State::Kept(_)
            )
        {
            return false;
        }
        let Some(action) = (match &self.state {
            State::Idle => return true,
            State::Requested(_, action)
            | State::Running(action)
            | State::Kept(action)
            | State::Restoring(action) => Some(*action),
        }) else {
            return true;
        };
        let pid = self.target.map(|target| target.pid);
        view.holdings.iter().all(|held| {
            held.action == Some(action)
                && held.pid == pid
                && self
                    .transaction
                    .as_ref()
                    .is_none_or(|id| *id == held.experiment_id)
        })
    }

    /// The restoration is accepted only when the worker also holds no token.
    fn restore_if_proven(&mut self, action: ResearchAction, view: &GateView) -> State {
        // A zero token count is the worker's word; the target's own kernel state must also be the
        // original one.
        let restored = match (&self.state_probe, &self.baseline) {
            // Qualifying evidence names its experiment: a restoration that cannot be attributed to one
            // is never accepted.
            (Some(_), Some(_)) if self.transaction.is_none() => return State::Restoring(action),
            (Some(probe), Some((target, original))) => {
                match probe(u32::from(target.pid)).filter(|now| now.restored_to(original)) {
                    Some(now) => Some((*target, original.clone(), now)),
                    None => return State::Restoring(action),
                }
            }
            _ => None,
        };
        if view.outstanding_tokens == Some(0)
            && self
                .with_scope(|scope| scope.restored(true))
                .unwrap_or(false)
        {
            if let Some((target, original, restored)) = restored {
                self.receipts.push(ActionReceipt {
                    transaction: self.transaction.take(),
                    action,
                    target,
                    original,
                    restored,
                });
            }
            State::Idle
        } else {
            State::Restoring(action)
        }
    }

    pub fn evidence(&self) -> GateEvidence {
        GateEvidence {
            settled: self
                .carried
                .iter()
                .copied()
                .chain(
                    self.scope
                        .as_ref()
                        .into_iter()
                        .flat_map(|scope| scope.settled().iter().copied()),
                )
                .collect(),
            clean: self.scope.as_ref().is_none_or(ActionChildScope::is_clean)
                && matches!(self.state, State::Idle)
                && !self.foreign,
            kernel_state_proven: self.state_probe.is_some()
                && !self.foreign
                && !self.receipts.is_empty()
                && self.receipts.len()
                    == self.carried.len()
                        + self.scope.as_ref().map_or(0, |scope| scope.settled().len()),
            receipts: self.receipts.clone(),
        }
    }
}

/// The pid's identity (start time and owner) from `/proc`, which a candidate must match to be the
/// frozen game process; a reused pid or a vanished process is not.
pub fn process_identity(pid: u32) -> Option<ChildTarget> {
    use std::os::unix::fs::MetadataExt;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the parenthesised command name; start time is field 22 overall.
    let after = stat.rsplit_once(')')?.1;
    let start_ticks: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let uid = std::fs::metadata(format!("/proc/{pid}")).ok()?.uid();
    Some(ChildTarget {
        pid: stutter_core::ids::Pid::new(pid),
        start_ticks,
        uid,
    })
}

#[cfg(test)]
#[path = "action_gate_tests.rs"]
mod tests;
