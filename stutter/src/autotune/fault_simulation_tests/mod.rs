//! Deterministic fault simulation (M9). It drives the real controller decision function, the real
//! live-experiment manager (start, keep, revert, fault), the real controller journal, real startup
//! recovery and real rollback-on-exit against a simulated host, and checks the same invariants after
//! every event: no unrecovered token, no conflicting kept groups, nothing applied outside the
//! support matrix, cooldowns honoured, every mutation audited. Time is virtual; nothing sleeps.
mod fixtures;
mod host;
mod invariants;
mod scenarios;

use std::{path::PathBuf, time::Duration};

use host::{Fault, SharedHost, SimExecutor, SimHost, SimRecoveryExecutor};

use crate::{
    actions::SafetyClass,
    autotune::{
        candidate_memory::{CandidateMemory, persistence::candidate_memory_path},
        conflicts::ActionConflictGroup,
        controller::{ControllerPolicy, ControllerRuntimeState, decide_autotune_transition},
        decision::AutotuneDecision,
        kept::ActiveProfileState,
        live_experiment::{
            LiveExperimentManager, LiveExperimentManagerInput, LiveExperimentRuntimeState,
        },
        objective::{ObjectiveSignals, runnable_signal::complete_runnable_population_for_tests},
        observation::AutotuneObservation,
        planning::candidate::CandidateAction,
        quality::OnlineDataQuality,
        situation::SituationKind,
        startup_recovery::{
            StartupRecoveryConfig, StartupRecoveryOutcome, recover_controller_journal_with_executor,
        },
        washout::WashoutWindowConfig,
    },
    daemon::{DaemonPolicy, policy::DaemonMode},
    daemon_policy::ActionSource,
    focus::FocusGroupKind,
    scorer::StutterScore,
};

const STEP: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Proposal {
    /// CPU affinity of the game: a graduated tuple (low risk, CpuPlacement).
    Affinity,
    /// cgroup placement of the game: a graduated tuple (medium risk, CgroupPlacement).
    Cgroup,
    /// Nice is not graduated: the support gate must refuse it before any write.
    Nice,
    /// A test-only non-conflicting candidate standing for a future graduated family, so the kept set
    /// can hold several actions at once.
    Coexisting(u8),
}

impl Proposal {
    fn candidate(self) -> CandidateAction {
        match self {
            Self::Affinity => fixtures::affinity(),
            Self::Cgroup => fixtures::cgroup(),
            Self::Nice => fixtures::nice(),
            Self::Coexisting(index) => CandidateAction::fake(
                crate::actions::ActionId::new(format!("sim-coexisting-{index}")),
                SafetyClass::ReversibleLowRisk,
            ),
        }
    }
}

/// How the workload responds while a candidate is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Response {
    Improves,
    Neutral,
    Regresses,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    /// One controller evaluation, optionally offered a candidate by the planner.
    Tick(Option<Proposal>),
    Respond(Response),
    WorkloadStop,
    WorkloadStart,
    Focus(bool),
    External(ActionConflictGroup),
    Fault(Fault),
    WorkerRestart,
    /// The controller process dies and restarts: volatile state is lost, startup recovery runs.
    DaemonRestart,
    /// A clean stop: the active experiment reverts and kept actions roll back.
    Shutdown,
    /// The machine sleeps: the gap invalidates any active window.
    Suspend(Duration),
    /// The machine reboots: runtime resources reset, the controller restarts.
    Reboot,
}

/// One audited transition, as the product reported it.
#[derive(Clone, Debug)]
pub(super) struct Audited {
    event: Event,
    pub(super) what: String,
    /// Host mutations this transition accounts for.
    mutations: usize,
}

pub(super) struct Sim {
    pub(super) dir: PathBuf,
    pub(super) host: SharedHost,
    pub(super) manager: LiveExperimentManager,
    pub(super) state: ControllerRuntimeState,
    pub(super) profile: ActiveProfileState,
    pub(super) policy: ControllerPolicy,
    daemon_policy: DaemonPolicy,
    pub(super) now: u128,
    pub(super) response: Response,
    target_present: bool,
    focus_game: bool,
    pub(super) trail: Vec<Audited>,
    pub(super) audited_mutations: usize,
    /// Invariant breaches observed while an event ran (cooldown and same-action spacing).
    pub(super) violations: Vec<String>,
    /// Last start time per action id, for the same-action spacing check.
    pub(super) last_started: std::collections::BTreeMap<String, u128>,
    /// Restarts since boot; a dead daemon loses candidate memory unless it is persisted.
    pub(super) restarts: usize,
    /// False while startup recovery has faulted: the agent refuses to start, so nothing runs.
    pub(super) up: bool,
}

impl Sim {
    pub(super) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "stutter-fault-sim-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).expect("simulation directory");
        let daemon_policy = DaemonPolicy::apply_medium_risk(ActionSource::AutotuneRuntime);
        Self {
            dir,
            host: SimHost::shared(),
            manager: LiveExperimentManager::new(),
            state: ControllerRuntimeState::default(),
            profile: ActiveProfileState::default(),
            policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
            daemon_policy,
            now: 1_000_000_000_000,
            response: Response::Improves,
            target_present: true,
            focus_game: true,
            trail: Vec::new(),
            audited_mutations: 0,
            violations: Vec::new(),
            last_started: std::collections::BTreeMap::new(),
            restarts: 0,
            up: true,
        }
    }

    /// The audited trail, one line per event, for a failing assertion.
    pub(super) fn report(&self) -> String {
        self.trail
            .iter()
            .map(|audited| {
                format!(
                    "{:?}: {} [{} mutation(s)]",
                    audited.event, audited.what, audited.mutations
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn journal_path(&self) -> PathBuf {
        self.dir.join("controller_journal.json")
    }

    fn memory_path(&self) -> PathBuf {
        candidate_memory_path(&self.journal_path())
    }

    fn save_memory(&self) {
        self.state
            .candidate_memory
            .save(&self.memory_path())
            .expect("candidate memory persists in the simulation");
    }

    fn input(&self) -> LiveExperimentManagerInput<'static> {
        LiveExperimentManagerInput {
            mode: DaemonMode::ApplyMediumRisk,
            daemon_policy: self.daemon_policy.clone(),
            controller_policy: self.policy.clone(),
            simulate_action_effects: false,
            washout: WashoutWindowConfig::default(),
            candidate_window_seconds: 30,
            manual_restore_command: "stutter daemon emergency-restore",
            audit_path: Some(self.dir.join("audit.jsonl")),
            controller_journal_path: Some(self.journal_path()),
            exit_rollback_registry: None,
            privileged_action_service: None,
        }
    }

    /// The baseline runnable p99 halves twice per kept improvement, so each experiment compares
    /// against what the previous keep achieved.
    fn baseline_p99(&self) -> u64 {
        8_000_000_u64 >> (2 * self.profile.kept_action_count().min(3))
    }

    fn observation(&self) -> AutotuneObservation {
        let active = self.manager.has_active_experiment();
        let p99 = match (active, self.response) {
            (true, Response::Improves) => self.baseline_p99() / 4,
            (true, Response::Regresses) => self.baseline_p99() * 4,
            _ => self.baseline_p99(),
        };
        let over_5ms = if p99 > 5_000_000 { 400 } else { 0 };
        let score = StutterScore {
            total: p99 / 1_000,
            frame_p99_ms: 12.0,
            frame_max_ms: 12.0,
            over_5ms,
            ..StutterScore::default()
        };
        let window = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 10,
            scored_samples: 2_000,
            scored_task_count: 1,
            score: score.clone(),
        };
        let mut signals = ObjectiveSignals::from_window_score(&window);
        complete_runnable_population_for_tests(&mut signals);
        signals.runnable_latency_p99_ns = Some(p99);
        signals.runnable_over_5ms = Some(over_5ms);
        AutotuneObservation {
            now_unix_nanos: self.now,
            target_present: self.target_present,
            target_root_pid: Some(1234),
            active_target_count: 1,
            interval_count: 10,
            scored_samples: 2_000,
            scored_task_count: 1,
            score,
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::GameCpuSchedulerPressure,
            focus_kind: Some(if self.focus_game {
                FocusGroupKind::Game
            } else {
                FocusGroupKind::Unknown
            }),
            focus_confidence: 1.0,
            focus_roots: vec![1234],
            objective_signals: signals,
            ..AutotuneObservation::default()
        }
    }

    fn record(&mut self, event: Event, what: impl Into<String>, mutations: usize) {
        self.audited_mutations += mutations;
        self.trail.push(Audited {
            event,
            what: what.into(),
            mutations,
        });
    }

    /// One controller evaluation, exactly the runtime's two calls: decide, then side effects.
    fn tick(&mut self, event: Event, proposal: Option<Proposal>) {
        self.now += STEP.as_nanos();
        let observation = self.observation();
        if !observation.target_present
            && !self.manager.has_active_experiment()
            && self.profile.kept_action_count() > 0
        {
            // The runtime releases kept actions once the workload they were kept for is gone.
            self.release_kept(event, "workload exited");
        }
        if let Some(window) = self.manager.active_window_decision(&observation) {
            self.record(event, format!("window: {window:?}"), 0);
            return;
        }
        let candidate = proposal.map(Proposal::candidate).filter(|candidate| {
            // The planner never offers a candidate that conflicts with a kept action.
            self.profile.conflicting_kept_action(candidate).is_none()
        });
        let decision =
            decide_autotune_transition(&self.policy, &self.state, &observation, candidate);
        let reason = format!("{decision:?}");
        let before = self.host.borrow().mutations;
        let cooldown = self.state.cooldown_until_unix_nanos;
        let started = match &decision {
            AutotuneDecision::StartExperiment { candidate, .. } => Some(candidate.clone()),
            _ => None,
        };
        let mut executor = SimExecutor {
            host: self.host.clone(),
        };
        let result = self.manager.apply_decision_side_effects_with_executor(
            self.input(),
            LiveExperimentRuntimeState {
                controller_state: &mut self.state,
                active_profile_state: &mut self.profile,
            },
            &observation,
            &decision,
            &reason,
            &mut executor,
        );
        self.save_memory();
        let mutated = self.host.borrow().mutations - before;
        match result {
            Ok(outcome) => {
                if outcome.event == crate::autotune::live_experiment::LiveExperimentEvent::Started
                    && let Some(candidate) = &started
                {
                    self.check_start(candidate, cooldown);
                }
                self.record(event, outcome.event.as_str(), mutated);
            }
            Err(error) => {
                // A refused or failed side effect is itself an audited fault.
                let context = self.manager.take_failed_apply_context();
                self.record(
                    event,
                    format!("error({}): {error:#}", context.is_some()),
                    mutated,
                );
            }
        }
    }

    pub(super) fn run(&mut self, event: Event) -> Result<(), String> {
        if !self.up && matches!(event, Event::Tick(_) | Event::Suspend(_) | Event::Shutdown) {
            self.record(event, "controller down: startup recovery faulted", 0);
            return self.check();
        }
        match event {
            Event::Tick(proposal) => self.tick(event, proposal),
            Event::Respond(response) => {
                self.response = response;
                self.record(event, "workload response changed", 0);
            }
            Event::WorkloadStop => {
                self.target_present = false;
                self.record(event, "workload stopped", 0);
            }
            Event::WorkloadStart => {
                self.target_present = true;
                self.record(event, "workload started", 0);
            }
            Event::Focus(game) => {
                self.focus_game = game;
                self.record(event, "focus changed", 0);
            }
            Event::External(group) => {
                self.host.borrow_mut().external_write(group);
                self.record(event, "external writer changed a controlled resource", 0);
            }
            Event::Fault(fault) => {
                self.host.borrow_mut().inject(fault);
                self.record(event, format!("fault injected: {fault:?}"), 0);
            }
            Event::WorkerRestart => {
                self.host.borrow_mut().restart_worker();
                self.record(event, "privileged worker restarted", 0);
            }
            Event::DaemonRestart => self.restart(event),
            Event::Shutdown => self.shutdown(event),
            Event::Suspend(gap) => {
                self.now += gap.as_nanos();
                if self.manager.has_active_experiment() {
                    // The runtime reverts an active candidate across a suspend gap.
                    let observation = self.observation();
                    let decision = AutotuneDecision::Revert {
                        experiment_id: self
                            .manager
                            .current_experiment_id()
                            .expect("active experiment has an id"),
                        reason: "suspend gap invalidated the measurement window".to_owned(),
                    };
                    self.side_effect(event, &observation, &decision);
                } else {
                    self.record(event, "suspend/resume with no active experiment", 0);
                }
            }
            Event::Reboot => {
                self.host.borrow_mut().reboot();
                self.restart(event);
            }
        }
        self.check()
    }

    fn side_effect(
        &mut self,
        event: Event,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
    ) {
        let before = self.host.borrow().mutations;
        let mut executor = SimExecutor {
            host: self.host.clone(),
        };
        let result = self.manager.apply_decision_side_effects_with_executor(
            self.input(),
            LiveExperimentRuntimeState {
                controller_state: &mut self.state,
                active_profile_state: &mut self.profile,
            },
            observation,
            decision,
            "simulation",
            &mut executor,
        );
        let mutated = self.host.borrow().mutations - before;
        match result {
            Ok(outcome) => self.record(event, outcome.event.as_str(), mutated),
            Err(error) => self.record(event, format!("error: {error:#}"), mutated),
        }
    }

    /// The process dies: memory is gone, durable state stays, startup recovery runs.
    fn restart(&mut self, event: Event) {
        self.restarts += 1;
        self.manager = LiveExperimentManager::new();
        self.state = ControllerRuntimeState::default();
        self.state.candidate_memory = CandidateMemory::load(&self.memory_path());
        self.profile = ActiveProfileState::default();
        let config = StartupRecoveryConfig {
            rollback_on_crash_recovery: true,
            journal_path: self.journal_path(),
            audit_path: self.dir.join("audit.jsonl"),
            history_path: self.dir.join("history.jsonl"),
            state_snapshot_path: self.dir.join("daemon-state.json"),
            privileged_worker_socket: None,
        };
        let before = self.host.borrow().mutations;
        let mut executor = SimRecoveryExecutor {
            host: self.host.clone(),
        };
        let outcome = recover_controller_journal_with_executor(config, &mut executor);
        let mutated = self.host.borrow().mutations - before;
        self.up = matches!(&outcome, Ok(other) if !other.is_faulted());
        let what = match outcome {
            Ok(StartupRecoveryOutcome::Clean) => "recovery: clean".to_owned(),
            Ok(other) => format!("recovery: {other:?}"),
            Err(error) => format!("recovery error: {error:#}"),
        };
        self.record(event, what, mutated);
    }

    /// A clean stop: revert the active experiment, then roll kept actions back on exit.
    fn shutdown(&mut self, event: Event) {
        if self.manager.has_active_experiment() {
            let observation = self.observation();
            let decision = AutotuneDecision::Revert {
                experiment_id: self
                    .manager
                    .current_experiment_id()
                    .expect("active experiment has an id"),
                reason: "shutdown".to_owned(),
            };
            self.side_effect(event, &observation, &decision);
        }
        self.release_kept(event, "shutdown");
    }

    /// The product's kept-action release: restore each kept action, retire it only on success.
    fn release_kept(&mut self, event: Event, reason: &str) {
        let observation = self.observation();
        let before = self.host.borrow().mutations;
        let mut executor = SimExecutor {
            host: self.host.clone(),
        };
        let release = LiveExperimentManager::release_kept_actions_with_executor(
            &self.input(),
            &mut self.profile,
            &observation,
            reason,
            &mut executor,
        );
        let mutated = self.host.borrow().mutations - before;
        self.record(
            event,
            format!(
                "kept release ({reason}) restored={} failed={}",
                release.restored, release.failed
            ),
            mutated,
        );
    }

    /// A start must not happen inside a cooldown or inside the same action's minimum spacing.
    fn check_start(&mut self, candidate: &CandidateAction, cooldown: Option<u128>) {
        if let Some(until) = cooldown
            && self.now < until
        {
            self.violations.push(format!(
                "experiment started {}ns before the cooldown ended",
                until - self.now
            ));
        }
        let action = candidate.action_id().as_str().to_owned();
        let spacing = self.policy.minimum_time_between_same_action.as_nanos();
        if let Some(last) = self.last_started.insert(action.clone(), self.now)
            && self.now < last + spacing
        {
            self.violations.push(format!(
                "action {action} restarted inside its minimum spacing"
            ));
        }
    }
}
