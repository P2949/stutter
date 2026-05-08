#![allow(dead_code)]
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    autotune::state::SituationKind, cli::FocusSource, foreground::ForegroundWindowSnapshot,
};

const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
const SCHED_DEADLINE: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SystemTaskClass {
    Game,
    GameRenderThread,
    GameWorkerThread,
    WineServer,
    GameScope,
    Compositor,

    AudioRealtime,
    Input,

    BrowserForeground,
    BrowserBackground,
    BrowserRenderer,
    BrowserGpu,
    BrowserNetwork,

    BuildJob,
    Compiler,
    Linker,
    Indexer,
    PackageManager,

    StorageDaemon,
    NetworkDaemon,
    KernelThread,
    IrqThread,

    Editor,
    Terminal,
    Shell,
    Media,
    Recorder,
    VirtualMachine,

    Service,

    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PriorityBand {
    CriticalRealtime,
    ForegroundLatency,
    Interactive,
    Throughput,
    Background,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub class: SystemTaskClass,
    pub priority_band: PriorityBand,
    pub confidence: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessIdentity<'a> {
    pub pid: u32,
    pub ppid: u32,
    pub comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: Option<&'a str>,
    pub cgroup_path: Option<&'a str>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ThreadIdentity<'a> {
    pub tid: u32,
    pub process_pid: u32,
    pub process_class: SystemTaskClass,
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FocusSnapshot {
    pub elapsed_ms: u64,
    pub foreground: Option<ForegroundWindowSnapshot>,
    pub processes: BTreeMap<u32, FocusProcess>,
    pub children_by_parent: BTreeMap<u32, Vec<u32>>,
    pub groups: Vec<FocusGroup>,
}

#[derive(Debug, Clone)]
pub struct FocusProcess {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cgroup_path: Option<PathBuf>,
    pub starttime_ticks: Option<u64>,
    pub sched_policy: Option<u32>,
    pub is_foreground_window_process: bool,

    pub classification: Classification,

    pub cpu_time_ticks_delta: u64,
    pub read_bytes_delta: u64,
    pub write_bytes_delta: u64,
    pub voluntary_ctxt_switches_delta: u64,
    pub nonvoluntary_ctxt_switches_delta: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusGroupKind {
    Game,
    Browser,
    Compile,
    Media,
    Recording,
    VirtualMachine,
    Desktop,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FocusScoreBreakdown {
    pub cpu_score: f32,
    pub io_score: f32,
    pub interactivity_score: f32,
    pub class_priority_score: f32,
    pub stability_score: f32,
    #[serde(default)]
    pub foreground_score: f32,
    pub penalty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SafetyWarning {
    CriticalRealtimePresent { pid: u32, comm: String },
    CompositorInFocusGroup { pid: u32, comm: String },
    UnknownForegroundLike { pid: u32, comm: String },
    TooBroadSystemServiceGroup { root_pids: Vec<u32> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusGroup {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub primary_pid: Option<u32>,
    pub display_name: String,
    pub score: f32,
    pub score_breakdown: FocusScoreBreakdown,
    pub confidence: f32,
    pub priority_band: PriorityBand,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FocusCache {
    previous: BTreeMap<u32, FocusCounters>,
    proc_cache: crate::process_tree::ProcessCache,
}

#[derive(Debug, Clone, Default)]
pub struct FocusCounters {
    pub starttime_ticks: Option<u64>,
    pub cpu_time_ticks: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub voluntary_ctxt_switches: u64,
    pub nonvoluntary_ctxt_switches: u64,
}

impl FocusCache {
    pub fn clear(&mut self) {
        self.previous.clear();
        self.proc_cache = crate::process_tree::ProcessCache::default();
    }
}

#[derive(Debug, Clone)]
pub struct FocusPolicy {
    pub poll_ms: u64,
    pub min_confidence: f32,
    pub switch_margin: f32,
    pub switch_cooldown_ms: u64,
    pub required_winner_polls: u32,
    pub max_roots: usize,
}

impl Default for FocusPolicy {
    fn default() -> Self {
        Self {
            poll_ms: 1000,
            min_confidence: 0.60,
            switch_margin: 0.20,
            switch_cooldown_ms: 5000,
            required_winner_polls: 2,
            max_roots: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFocus {
    pub group: FocusGroup,
    pub selected_at_ms: u64,
    pub last_confirmed_ms: u64,
    pub situation: SituationKind,
}

#[derive(Debug, Clone)]
struct PendingFocus {
    group: FocusGroup,
    first_seen_ms: u64,
    polls: u32,
}

pub struct FocusResolver {
    cache: FocusCache,
    current: Option<ResolvedFocus>,
    pending: Option<PendingFocus>,
    policy: FocusPolicy,
}

#[derive(Debug, Clone)]
pub enum FocusDecision {
    Keep {
        focus: ResolvedFocus,
    },
    Switch {
        old: Option<ResolvedFocus>,
        new: ResolvedFocus,
    },
    Clear {
        old: Option<ResolvedFocus>,
        reason: String,
    },
    NoTarget {
        reason: String,
    },
}

impl FocusResolver {
    pub fn new(policy: FocusPolicy) -> Self {
        Self {
            cache: FocusCache::default(),
            current: None,
            pending: None,
            policy,
        }
    }

    pub fn sample(
        &mut self,
        proc_root: &Path,
        elapsed_ms: u64,
        foreground: Option<&ForegroundWindowSnapshot>,
        source_mode: FocusSource,
    ) -> FocusDecision {
        let mut snapshot = focus_snapshot_at(proc_root, &mut self.cache, elapsed_ms, foreground);
        apply_foreground_source_mode_to_snapshot(&mut snapshot, source_mode);
        self.decide_from_snapshot(snapshot)
    }

    pub fn decide_from_snapshot(&mut self, snapshot: FocusSnapshot) -> FocusDecision {
        let candidate = self.best_eligible_group(&snapshot);

        let Some(candidate) = candidate else {
            self.pending = None;
            if let Some(old) = self.current.take() {
                return FocusDecision::Clear {
                    old: Some(old),
                    reason: format!(
                        "no focus group confidence met min_confidence={:.2}",
                        self.policy.min_confidence
                    ),
                };
            }

            return FocusDecision::NoTarget {
                reason: format!(
                    "no focus group confidence met min_confidence={:.2}",
                    self.policy.min_confidence
                ),
            };
        };

        let candidate = self.limit_group_roots(candidate);

        if let Some(existing_current) = self.current.clone() {
            let current_group = snapshot
                .groups
                .iter()
                .find(|group| Self::same_focus_identity(group, &existing_current.group))
                .cloned()
                .map(|group| self.limit_group_roots(group));

            if let Some(current_group) =
                current_group.filter(|g| Self::group_roots_alive(&snapshot, g) && g.score >= 0.45)
            {
                let mut refreshed = existing_current.clone();
                refreshed.group = current_group;
                refreshed.last_confirmed_ms = snapshot.elapsed_ms;
                refreshed.situation = Self::situation_for_group(&refreshed.group);

                if Self::same_focus_identity(&refreshed.group, &candidate) {
                    self.current = Some(refreshed.clone());
                    self.pending = None;
                    return FocusDecision::Keep { focus: refreshed };
                }

                if candidate.score < refreshed.group.score + self.policy.switch_margin {
                    self.current = Some(refreshed.clone());
                    self.pending = None;
                    return FocusDecision::Keep { focus: refreshed };
                }

                if snapshot.elapsed_ms.saturating_sub(refreshed.selected_at_ms)
                    < self.policy.switch_cooldown_ms
                {
                    self.current = Some(refreshed.clone());
                    self.pending = None;
                    return FocusDecision::Keep { focus: refreshed };
                }

                if !self.confirm_pending_winner(&candidate, snapshot.elapsed_ms) {
                    self.current = Some(refreshed.clone());
                    return FocusDecision::Keep { focus: refreshed };
                }

                let old = Some(refreshed);
                let new = Self::resolved_focus_from_group(candidate, snapshot.elapsed_ms);
                self.current = Some(new.clone());
                self.pending = None;
                return FocusDecision::Switch { old, new };
            }

            if !self.confirm_pending_winner(&candidate, snapshot.elapsed_ms) {
                let old = self.current.take();
                return FocusDecision::Clear {
                    old,
                    reason:
                        "current focus root disappeared or score fell below 0.45; waiting for a stable replacement winner"
                            .to_owned(),
                };
            }

            let old = self.current.take();
            let new = Self::resolved_focus_from_group(candidate, snapshot.elapsed_ms);
            self.current = Some(new.clone());
            self.pending = None;
            return FocusDecision::Switch { old, new };
        }

        if !self.confirm_pending_winner(&candidate, snapshot.elapsed_ms) {
            return FocusDecision::NoTarget {
                reason: format!(
                    "waiting for stable winner poll {}/{}",
                    self.pending
                        .as_ref()
                        .map(|pending| pending.polls)
                        .unwrap_or(0),
                    self.policy.required_winner_polls.max(1)
                ),
            };
        }

        let new = Self::resolved_focus_from_group(candidate, snapshot.elapsed_ms);
        self.current = Some(new.clone());
        self.pending = None;
        FocusDecision::Switch { old: None, new }
    }

    fn best_eligible_group(&self, snapshot: &FocusSnapshot) -> Option<FocusGroup> {
        snapshot
            .groups
            .iter()
            .filter(|group| group.confidence >= self.policy.min_confidence)
            .filter(|group| Self::group_roots_alive(snapshot, group))
            .cloned()
            .max_by(Self::compare_group_preference)
    }

    fn confirm_pending_winner(&mut self, candidate: &FocusGroup, elapsed_ms: u64) -> bool {
        let required = self.policy.required_winner_polls.max(1);

        if required == 1 {
            self.pending = Some(PendingFocus {
                group: candidate.clone(),
                first_seen_ms: elapsed_ms,
                polls: 1,
            });
            return true;
        }

        match self.pending.as_mut() {
            Some(pending) if Self::same_focus_identity(&pending.group, candidate) => {
                pending.group = candidate.clone();
                pending.polls = pending.polls.saturating_add(1);
                pending.polls >= required
            }
            _ => {
                self.pending = Some(PendingFocus {
                    group: candidate.clone(),
                    first_seen_ms: elapsed_ms,
                    polls: 1,
                });
                false
            }
        }
    }

    fn resolved_focus_from_group(group: FocusGroup, elapsed_ms: u64) -> ResolvedFocus {
        let situation = Self::situation_for_group(&group);
        ResolvedFocus {
            group,
            selected_at_ms: elapsed_ms,
            last_confirmed_ms: elapsed_ms,
            situation,
        }
    }

    fn situation_for_group(group: &FocusGroup) -> SituationKind {
        crate::focus::situation_for_group(group)
    }

    fn limit_group_roots(&self, mut group: FocusGroup) -> FocusGroup {
        if group.root_pids.len() > self.policy.max_roots {
            group.root_pids.truncate(self.policy.max_roots);
        }
        group
    }

    fn group_roots_alive(snapshot: &FocusSnapshot, group: &FocusGroup) -> bool {
        if group.root_pids.is_empty() {
            return group
                .primary_pid
                .map(|pid| snapshot.processes.contains_key(&pid))
                .unwrap_or(false);
        }

        group
            .root_pids
            .iter()
            .any(|pid| snapshot.processes.contains_key(pid))
    }

    fn same_focus_identity(left: &FocusGroup, right: &FocusGroup) -> bool {
        if left.kind != right.kind {
            return false;
        }

        if !left.root_pids.is_empty() && !right.root_pids.is_empty() {
            return left
                .root_pids
                .iter()
                .any(|pid| right.root_pids.contains(pid));
        }

        match (left.primary_pid, right.primary_pid) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }

    fn compare_group_preference(left: &FocusGroup, right: &FocusGroup) -> std::cmp::Ordering {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.confidence
                    .partial_cmp(&right.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                priority_band_rank(left.priority_band).cmp(&priority_band_rank(right.priority_band))
            })
            .then_with(|| {
                let left_root = left
                    .root_pids
                    .first()
                    .copied()
                    .or(left.primary_pid)
                    .unwrap_or(u32::MAX);
                let right_root = right
                    .root_pids
                    .first()
                    .copied()
                    .or(right.primary_pid)
                    .unwrap_or(u32::MAX);
                right_root.cmp(&left_root)
            })
    }
}

#[cfg(test)]
mod foreground_focus_tests {
    use super::*;

    fn foreground_snapshot(pid: Option<u32>) -> ForegroundWindowSnapshot {
        ForegroundWindowSnapshot {
            elapsed_ms: 1_000,
            source: Some(crate::foreground::ForegroundSource::Sway),
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid,
            app_id: Some("steam".to_owned()),
            class: Some("Steam".to_owned()),
            title: None,
            window_id: Some("7".to_owned()),
            workspace: Some("games".to_owned()),
            confidence: 0.95,
            stale_ms: None,
            reason: "test foreground snapshot".to_owned(),
        }
    }

    fn test_classification(
        class: SystemTaskClass,
        priority_band: PriorityBand,
        confidence: f32,
    ) -> Classification {
        Classification {
            class,
            priority_band,
            confidence,
            reasons: vec![format!("test class {:?}", class)],
        }
    }

    fn test_process(pid: u32, ppid: u32, class: SystemTaskClass) -> FocusProcess {
        FocusProcess {
            pid,
            ppid,
            comm: format!("process-{pid}"),
            cmdline: format!("process-{pid}"),
            cgroup_path: None,
            starttime_ticks: Some(pid as u64 * 10),
            sched_policy: None,
            is_foreground_window_process: false,
            classification: test_classification(class, PriorityBand::Interactive, 0.85),
            cpu_time_ticks_delta: 10,
            read_bytes_delta: 0,
            write_bytes_delta: 0,
            voluntary_ctxt_switches_delta: 0,
            nonvoluntary_ctxt_switches_delta: 0,
        }
    }

    fn test_group(
        kind: FocusGroupKind,
        root_pids: Vec<u32>,
        member_pids: Vec<u32>,
        primary_pid: Option<u32>,
        display_name: &str,
        score: f32,
    ) -> FocusGroup {
        FocusGroup {
            kind,
            root_pids,
            member_pids,
            primary_pid,
            display_name: display_name.to_owned(),
            score,
            score_breakdown: FocusScoreBreakdown {
                cpu_score: 0.50,
                io_score: 0.10,
                interactivity_score: 0.50,
                class_priority_score: 0.50,
                stability_score: 0.50,
                foreground_score: 0.0,
                penalty: 0.0,
            },
            confidence: 0.80,
            priority_band: PriorityBand::Interactive,
            reasons: vec![format!("test group {display_name}")],
        }
    }

    fn foreground_scoring_snapshot(
        foreground: Option<ForegroundWindowSnapshot>,
        groups: Vec<FocusGroup>,
    ) -> FocusSnapshot {
        let mut processes = BTreeMap::new();
        processes.insert(10, test_process(10, 1, SystemTaskClass::Game));
        processes.insert(11, test_process(11, 10, SystemTaskClass::GameWorkerThread));
        processes.insert(20, test_process(20, 1, SystemTaskClass::BrowserForeground));
        processes.insert(21, test_process(21, 20, SystemTaskClass::BrowserRenderer));
        processes.insert(30, test_process(30, 1, SystemTaskClass::Compiler));

        let mut children_by_parent = BTreeMap::new();
        children_by_parent.insert(1, vec![10, 20, 30]);
        children_by_parent.insert(10, vec![11]);
        children_by_parent.insert(20, vec![21]);

        FocusSnapshot {
            elapsed_ms: 1_000,
            foreground,
            processes,
            children_by_parent,
            groups,
        }
    }

    #[test]
    fn foreground_pid_boosts_matching_game_group() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.70,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        let game = snapshot
            .groups
            .iter()
            .find(|group| group.display_name == "game")
            .unwrap();
        assert!(game.score_breakdown.foreground_score > 0.0);
        assert!(game.score > 0.30);
        assert!(
            game.reasons
                .iter()
                .any(|reason| reason.contains("foreground-window score"))
        );
    }

    #[test]
    fn foreground_source_rejects_non_foreground_heuristic_winner() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.95,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "game");
        assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
    }

    #[test]
    fn hybrid_source_falls_back_when_provider_unavailable() {
        let mut unavailable = foreground_snapshot(Some(11));
        unavailable.status = crate::foreground::ForegroundProviderStatus::Unavailable;
        unavailable.confidence = 0.0;
        let mut snapshot = foreground_scoring_snapshot(
            Some(unavailable),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.95,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].display_name, "game");
        assert_eq!(snapshot.groups[0].score, 0.30);
        assert_eq!(snapshot.groups[1].display_name, "browser");
        assert_eq!(snapshot.groups[1].score, 0.95);
    }

    #[test]
    fn foreground_pid_does_not_select_system_service_root() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "systemd".to_owned();
        process.cmdline = "/usr/lib/systemd/systemd --user".to_owned();
        process.classification.class = SystemTaskClass::Service;

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_browser_maps_to_browser_foreground_band() {
        let foreground = foreground_snapshot(Some(20));
        let mut browser = test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20, 21],
            Some(20),
            "browser",
            0.20,
        );
        browser.priority_band = PriorityBand::ForegroundLatency;
        browser.confidence = 0.80;

        let mut snapshot = foreground_scoring_snapshot(Some(foreground), vec![browser]);
        snapshot
            .processes
            .get_mut(&20)
            .unwrap()
            .classification
            .priority_band = PriorityBand::ForegroundLatency;

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].kind, FocusGroupKind::Browser);
        assert_eq!(
            snapshot.groups[0].priority_band,
            PriorityBand::ForegroundLatency
        );
        assert_eq!(snapshot.groups[0].root_pids, vec![20]);
        assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
    }

    #[test]
    fn foreground_source_preserves_existing_group_roots_when_group_contains_foreground_pid() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 11],
                Some(10),
                "game-with-helper-root",
                0.40,
            )],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "game-with-helper-root");
        assert_eq!(snapshot.groups[0].root_pids, vec![10]);
        assert_eq!(snapshot.groups[0].member_pids, vec![10, 11]);
        assert!(
            snapshot.groups[0].reasons.iter().all(|reason| reason
                != "foreground window PID selected but no known focus group matched")
        );
    }

    #[test]
    fn foreground_source_adds_conservative_unknown_fallback_when_no_group_contains_foreground_pid()
    {
        let foreground = foreground_snapshot(Some(30));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );
        // Break family match between browser (PPID 1) and compiler (PID 30)
        snapshot.processes.get_mut(&30).unwrap().ppid = 99;

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].kind, FocusGroupKind::Unknown);
        assert_eq!(snapshot.groups[0].root_pids, vec![30]);
        assert_eq!(snapshot.groups[0].member_pids, Vec::<u32>::new());
        assert_eq!(snapshot.groups[0].primary_pid, Some(30));
        assert_eq!(snapshot.groups[0].display_name, "foreground:process-30");
        assert!((snapshot.groups[0].confidence - (0.95 * 0.75)).abs() < f32::EPSILON);
        assert!(
            snapshot.groups[0].reasons.iter().any(|reason| reason
                == "foreground window PID selected but no known focus group matched")
        );
        assert!(
            snapshot.groups[0]
                .reasons
                .iter()
                .any(|reason| reason == "conservative foreground fallback root pid=30")
        );
        assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
    }

    #[test]
    fn foreground_source_fallback_member_pids_are_descendants_not_the_root_pid() {
        let foreground = foreground_snapshot(Some(10));
        let mut snapshot = foreground_scoring_snapshot(Some(foreground), Vec::new());

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].root_pids, vec![10]);
        assert_eq!(snapshot.groups[0].member_pids, vec![11]);
        assert!(!snapshot.groups[0].member_pids.contains(&10));
    }

    #[test]
    fn foreground_source_does_not_auto_target_pid_one() {
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground_snapshot(Some(1))),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );
        snapshot
            .processes
            .insert(1, test_process(1, 0, SystemTaskClass::Service));

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_systemd() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "systemd".to_owned();
        process.cmdline = "/usr/lib/systemd/systemd --user".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_compositor() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.classification.class = SystemTaskClass::Compositor;
        process.classification.priority_band = PriorityBand::ForegroundLatency;

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_realtime_audio_or_input() {
        let mut audio_snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let audio_process = audio_snapshot.processes.get_mut(&30).unwrap();
        audio_process.classification.class = SystemTaskClass::AudioRealtime;
        audio_process.classification.priority_band = PriorityBand::CriticalRealtime;

        apply_foreground_source_mode_to_snapshot(&mut audio_snapshot, FocusSource::Foreground);

        assert!(audio_snapshot.groups.is_empty());

        let mut input_snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let input_process = input_snapshot.processes.get_mut(&30).unwrap();
        input_process.classification.class = SystemTaskClass::Input;
        input_process.classification.priority_band = PriorityBand::CriticalRealtime;

        apply_foreground_source_mode_to_snapshot(&mut input_snapshot, FocusSource::Foreground);

        assert!(input_snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_xwayland() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "Xwayland".to_owned();
        process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn hybrid_source_unsafe_foreground_falls_back_to_existing_heuristic_groups() {
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground_snapshot(Some(30))),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "Xwayland".to_owned();
        process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "browser");
        assert_eq!(snapshot.groups[0].root_pids, vec![20]);
        assert_eq!(snapshot.groups[0].score, 0.90);
        assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
    }

    #[test]
    fn foreground_score_is_full_confidence_for_member_pid() {
        let foreground = foreground_snapshot(Some(11));
        let snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 11],
                Some(10),
                "game",
                0.40,
            )],
        );

        let score = foreground_score_for_group(&snapshot.groups[0], &snapshot);

        assert!((score - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn foreground_score_is_partial_for_same_process_family() {
        let foreground = foreground_snapshot(Some(11));
        let snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10],
                Some(10),
                "game",
                0.40,
            )],
        );

        let score = foreground_score_for_group(&snapshot.groups[0], &snapshot);

        assert!((score - (0.75 * 0.95)).abs() < f32::EPSILON);
    }

    #[test]
    fn foreground_source_keeps_only_foreground_scoring_groups() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.90,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "game");
        assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
        assert!(snapshot.groups[0].score > 0.30);
    }

    #[test]
    fn foreground_source_falls_back_to_heuristic_when_provider_unavailable() {
        let mut unavailable = foreground_snapshot(Some(11));
        unavailable.status = crate::foreground::ForegroundProviderStatus::Unavailable;
        unavailable.confidence = 0.0;
        let mut snapshot = foreground_scoring_snapshot(
            Some(unavailable),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.90,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].display_name, "game");
        assert_eq!(snapshot.groups[0].score, 0.30);
        assert_eq!(snapshot.groups[1].display_name, "browser");
        assert_eq!(snapshot.groups[1].score, 0.90);
    }

    #[test]
    fn hybrid_source_boosts_foreground_group_but_keeps_fallback_groups() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.90,
                ),
            ],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        assert_eq!(snapshot.groups.len(), 2);
        assert!(
            snapshot
                .groups
                .iter()
                .any(|group| group.display_name == "game")
        );
        assert!(
            snapshot
                .groups
                .iter()
                .any(|group| group.display_name == "browser")
        );
        let game = snapshot
            .groups
            .iter()
            .find(|group| group.display_name == "game")
            .unwrap();
        assert!(game.score_breakdown.foreground_score > 0.0);
        assert!(
            game.reasons
                .iter()
                .any(|reason| reason.contains("foreground-window score"))
        );
    }

    #[test]
    fn hybrid_source_falls_back_to_heuristic_when_foreground_is_stale() {
        let mut stale = foreground_snapshot(Some(11));
        stale.stale_ms = Some(1_000);
        let mut snapshot = foreground_scoring_snapshot(
            Some(stale),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "browser");
        assert_eq!(snapshot.groups[0].score, 0.90);
        assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
    }

    #[test]
    fn heuristic_source_preserves_scores_exactly() {
        let foreground = foreground_snapshot(Some(11));
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground),
            vec![
                test_group(
                    FocusGroupKind::Game,
                    vec![10],
                    vec![10, 11],
                    Some(10),
                    "game",
                    0.30,
                ),
                test_group(
                    FocusGroupKind::Browser,
                    vec![20],
                    vec![20, 21],
                    Some(20),
                    "browser",
                    0.90,
                ),
            ],
        );
        let before = snapshot
            .groups
            .iter()
            .map(|group| {
                (
                    group.display_name.clone(),
                    group.score,
                    group.score_breakdown.foreground_score,
                )
            })
            .collect::<Vec<_>>();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Heuristic);

        let after = snapshot
            .groups
            .iter()
            .map(|group| {
                (
                    group.display_name.clone(),
                    group.score,
                    group.score_breakdown.foreground_score,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(before, after);
    }

    #[test]
    fn foreground_breakdown_deserializes_old_json_with_default_foreground_score() {
        let json = r#"{
            "cpu_score": 0.1,
            "io_score": 0.2,
            "interactivity_score": 0.3,
            "class_priority_score": 0.4,
            "stability_score": 0.5,
            "penalty": 0.6
        }"#;

        let breakdown: FocusScoreBreakdown = serde_json::from_str(json).unwrap();

        assert_eq!(breakdown.foreground_score, 0.0);
    }

    #[test]
    fn focus_snapshot_stores_foreground_snapshot() {
        let proc = crate::test_support::FakeProc::new("focus-stores-foreground");
        proc.write_process(
            crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
                .cmdline(vec!["steam_app_379430".to_owned()])
                .cgroup("/user.slice/app.slice"),
        )
        .unwrap();

        let foreground = foreground_snapshot(Some(4242));
        let mut cache = FocusCache::default();

        let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

        assert_eq!(
            snapshot.foreground.as_ref().and_then(|fg| fg.pid),
            Some(4242)
        );
        assert_eq!(
            snapshot
                .foreground
                .as_ref()
                .and_then(|fg| fg.app_id.as_deref()),
            Some("steam")
        );
    }

    #[test]
    fn focus_snapshot_marks_matching_foreground_process() {
        let proc = crate::test_support::FakeProc::new("focus-marks-matching-foreground");
        proc.write_process(
            crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
                .cmdline(vec!["steam_app_379430".to_owned()])
                .cgroup("/user.slice/app.slice"),
        )
        .unwrap();
        proc.write_process(
            crate::test_support::FakeProcess::new(9000, "firefox", 22_222)
                .cmdline(vec!["firefox".to_owned()])
                .cgroup("/user.slice/app.slice"),
        )
        .unwrap();

        let foreground = foreground_snapshot(Some(4242));
        let mut cache = FocusCache::default();

        let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

        assert!(
            snapshot
                .processes
                .get(&4242)
                .unwrap()
                .is_foreground_window_process
        );
        assert!(
            !snapshot
                .processes
                .get(&9000)
                .unwrap()
                .is_foreground_window_process
        );
    }

    #[test]
    fn focus_snapshot_does_not_mark_any_process_without_foreground_pid() {
        let proc = crate::test_support::FakeProc::new("focus-no-foreground-pid");
        proc.write_process(
            crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
                .cmdline(vec!["steam_app_379430".to_owned()])
                .cgroup("/user.slice/app.slice"),
        )
        .unwrap();

        let foreground = foreground_snapshot(None);
        let mut cache = FocusCache::default();

        let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

        assert!(
            !snapshot
                .processes
                .get(&4242)
                .unwrap()
                .is_foreground_window_process
        );
    }

    #[test]
    fn focus_resolver_sample_accepts_foreground_and_source_mode() {
        let proc = crate::test_support::FakeProc::new("focus-resolver-foreground-source-mode");
        proc.write_process(
            crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
                .cmdline(vec!["steam_app_379430".to_owned()])
                .cgroup("/user.slice/app.slice"),
        )
        .unwrap();

        let foreground = foreground_snapshot(Some(4242));
        let mut resolver = FocusResolver::new(FocusPolicy {
            min_confidence: 0.0,
            required_winner_polls: 1,
            ..FocusPolicy::default()
        });

        let decision = resolver.sample(
            proc.path(),
            1_000,
            Some(&foreground),
            FocusSource::Foreground,
        );

        match decision {
            FocusDecision::Keep { .. }
            | FocusDecision::Switch { .. }
            | FocusDecision::Clear { .. }
            | FocusDecision::NoTarget { .. } => {}
        }
    }
}

fn apply_foreground_source_mode_to_snapshot(
    snapshot: &mut FocusSnapshot,
    source_mode: FocusSource,
) {
    if source_mode == FocusSource::Heuristic {
        return;
    }

    let foreground_available = foreground_context_is_available_for_focus(snapshot);

    if !foreground_available {
        return;
    }

    if !foreground_target_is_safe_to_score(snapshot) {
        if source_mode == FocusSource::Foreground {
            snapshot.groups.clear();
        }
        return;
    }

    add_foreground_fallback_group_if_needed(snapshot);

    let mut groups = std::mem::take(&mut snapshot.groups);

    for group in groups.iter_mut() {
        let foreground_score = foreground_score_for_group(group, snapshot);
        group.score_breakdown.foreground_score = foreground_score;

        if foreground_score > 0.0 {
            group.score = foreground_aware_total_score(&group.score_breakdown);
            if group.confidence < foreground_score && !is_foreground_fallback_group(group) {
                group.confidence = foreground_score;
            }
            group
                .reasons
                .push(format!("foreground-window score {:.2}", foreground_score));
        } else if source_mode == FocusSource::Hybrid {
            group.score = foreground_aware_total_score(&group.score_breakdown);
        }
    }

    if source_mode == FocusSource::Foreground {
        groups.retain(|group| group.score_breakdown.foreground_score > 0.0);
    }

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                priority_band_rank(right.priority_band).cmp(&priority_band_rank(left.priority_band))
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    snapshot.groups = groups;
}

fn foreground_context_is_available_for_focus(snapshot: &FocusSnapshot) -> bool {
    let Some(foreground) = snapshot.foreground.as_ref() else {
        return false;
    };

    foreground.status == crate::foreground::ForegroundProviderStatus::Available
        && foreground.pid.is_some()
        && foreground.confidence > 0.0
        && foreground.stale_ms.is_none()
}

fn foreground_target_is_safe_to_score(snapshot: &FocusSnapshot) -> bool {
    let Some(pid) = snapshot
        .foreground
        .as_ref()
        .and_then(|foreground| foreground.pid)
    else {
        return false;
    };

    let Some(process) = snapshot.processes.get(&pid) else {
        return false;
    };

    foreground_process_is_safe_auto_target(process)
}

fn foreground_process_is_safe_auto_target(process: &FocusProcess) -> bool {
    if process.pid == 1 {
        return false;
    }

    if process_name_looks_like_systemd(process) {
        return false;
    }

    if process.classification.class == SystemTaskClass::Compositor {
        return false;
    }

    if is_critical_realtime_process(process) {
        return false;
    }

    if process_name_looks_like_xwayland(process) {
        return false;
    }

    true
}

fn process_name_looks_like_systemd(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();

    comm == "systemd"
        || comm.starts_with("systemd-")
        || cmdline == "systemd"
        || cmdline.contains("/systemd ")
        || cmdline.contains("/systemd\0")
        || cmdline.ends_with("/systemd")
}

fn process_name_looks_like_xwayland(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();

    comm == "xwayland" || comm == "xwayland.bin" || cmdline.contains("xwayland")
}

fn is_foreground_fallback_group(group: &FocusGroup) -> bool {
    group
        .reasons
        .iter()
        .any(|reason| reason == "foreground window PID selected but no known focus group matched")
}

fn add_foreground_fallback_group_if_needed(snapshot: &mut FocusSnapshot) {
    let Some(foreground) = snapshot.foreground.as_ref() else {
        return;
    };

    let Some(pid) = foreground.pid else {
        return;
    };

    if snapshot
        .groups
        .iter()
        .any(|group| focus_group_contains_pid(group, pid))
    {
        return;
    }

    let Some(process) = snapshot.processes.get(&pid) else {
        return;
    };

    if !foreground_process_is_safe_auto_target(process) {
        return;
    }

    let member_pids = descendants_of_process(snapshot, pid);
    let confidence = (foreground.confidence * 0.75).clamp(0.0, 1.0);
    let display_name = if process.comm.trim().is_empty() {
        format!("foreground:{pid}")
    } else {
        format!("foreground:{}", process.comm)
    };

    snapshot.groups.push(FocusGroup {
        kind: FocusGroupKind::Unknown,
        root_pids: vec![pid],
        member_pids,
        primary_pid: Some(pid),
        display_name,
        score: confidence,
        score_breakdown: FocusScoreBreakdown {
            cpu_score: 0.0,
            io_score: 0.0,
            interactivity_score: 0.0,
            class_priority_score: 0.0,
            stability_score: 0.0,
            foreground_score: 0.0,
            penalty: 0.0,
        },
        confidence,
        priority_band: process.classification.priority_band,
        reasons: vec![
            "foreground window PID selected but no known focus group matched".to_owned(),
            format!("conservative foreground fallback root pid={pid}"),
        ],
    });
}

fn focus_group_contains_pid(group: &FocusGroup, pid: u32) -> bool {
    group.root_pids.contains(&pid)
        || group.member_pids.contains(&pid)
        || group.primary_pid == Some(pid)
}

fn descendants_of_process(snapshot: &FocusSnapshot, root_pid: u32) -> Vec<u32> {
    let mut descendants = BTreeSet::new();
    let mut stack = snapshot
        .children_by_parent
        .get(&root_pid)
        .cloned()
        .unwrap_or_default();

    while let Some(pid) = stack.pop() {
        if !snapshot.processes.contains_key(&pid) {
            continue;
        }

        if !descendants.insert(pid) {
            continue;
        }

        if let Some(children) = snapshot.children_by_parent.get(&pid) {
            stack.extend(children.iter().copied());
        }
    }

    descendants.into_iter().collect()
}

fn foreground_aware_total_score(breakdown: &FocusScoreBreakdown) -> f32 {
    breakdown.cpu_score * 0.25
        + breakdown.io_score * 0.10
        + breakdown.interactivity_score * 0.15
        + breakdown.class_priority_score * 0.20
        + breakdown.stability_score * 0.10
        + breakdown.foreground_score * 0.35
        - breakdown.penalty
}

fn foreground_score_for_group(group: &FocusGroup, snapshot: &FocusSnapshot) -> f32 {
    let Some(fg) = snapshot.foreground.as_ref() else {
        return 0.0;
    };

    let Some(pid) = fg.pid else {
        return 0.0;
    };

    if group.member_pids.contains(&pid) {
        return 1.0 * fg.confidence;
    }

    if group.root_pids.contains(&pid) {
        return 0.95 * fg.confidence;
    }

    if group
        .member_pids
        .iter()
        .any(|member| same_process_family(snapshot, *member, pid))
    {
        return 0.75 * fg.confidence;
    }

    0.0
}

fn same_process_family(snapshot: &FocusSnapshot, left_pid: u32, right_pid: u32) -> bool {
    if left_pid == right_pid {
        return true;
    }

    if is_process_ancestor(snapshot, left_pid, right_pid) {
        return true;
    }

    if is_process_ancestor(snapshot, right_pid, left_pid) {
        return true;
    }

    let left_parent = snapshot
        .processes
        .get(&left_pid)
        .map(|process| process.ppid);
    let right_parent = snapshot
        .processes
        .get(&right_pid)
        .map(|process| process.ppid);

    matches!((left_parent, right_parent), (Some(left), Some(right)) if left != 0 && left == right)
}

fn is_process_ancestor(snapshot: &FocusSnapshot, ancestor_pid: u32, descendant_pid: u32) -> bool {
    let mut current = descendant_pid;
    let mut seen = BTreeSet::new();

    while seen.insert(current) {
        let Some(process) = snapshot.processes.get(&current) else {
            return false;
        };

        if process.ppid == ancestor_pid {
            return true;
        }

        if process.ppid == 0 || process.ppid == current {
            return false;
        }

        current = process.ppid;
    }

    false
}

pub fn safety_warnings_for_group(
    group: &FocusGroup,
    snapshot: &FocusSnapshot,
) -> Vec<SafetyWarning> {
    let mut warnings = Vec::new();

    for pid in &group.member_pids {
        let Some(process) = snapshot.processes.get(pid) else {
            continue;
        };

        if is_critical_realtime_process(process) {
            warnings.push(SafetyWarning::CriticalRealtimePresent {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }

        if process.classification.class == SystemTaskClass::Compositor {
            warnings.push(SafetyWarning::CompositorInFocusGroup {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }

        if is_unknown_foreground_like(process) {
            warnings.push(SafetyWarning::UnknownForegroundLike {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }
    }

    if is_too_broad_system_service_group(group, snapshot) {
        warnings.push(SafetyWarning::TooBroadSystemServiceGroup {
            root_pids: group.root_pids.clone(),
        });
    }

    warnings
}

fn is_critical_realtime_process(process: &FocusProcess) -> bool {
    matches!(
        process.classification.class,
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input
    ) || process.classification.priority_band == PriorityBand::CriticalRealtime
}

fn is_unknown_foreground_like(process: &FocusProcess) -> bool {
    process.classification.class == SystemTaskClass::Unknown
        && is_active_foreground_candidate(process)
        && process.classification.priority_band != PriorityBand::Background
}

fn is_too_broad_system_service_group(group: &FocusGroup, snapshot: &FocusSnapshot) -> bool {
    if group.kind != FocusGroupKind::Idle {
        return false;
    }

    let root_is_system_service = group.root_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_system_service_root)
            .unwrap_or(false)
    });

    let all_members_are_service_like = !group.member_pids.is_empty()
        && group.member_pids.iter().all(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| {
                    matches!(
                        process.classification.class,
                        SystemTaskClass::Service
                            | SystemTaskClass::StorageDaemon
                            | SystemTaskClass::NetworkDaemon
                            | SystemTaskClass::KernelThread
                            | SystemTaskClass::IrqThread
                    )
                })
                .unwrap_or(false)
        });

    root_is_system_service && (group.member_pids.len() >= 4 || all_members_are_service_like)
}

fn is_system_service_root(process: &FocusProcess) -> bool {
    if !matches!(
        process.classification.class,
        SystemTaskClass::Service
            | SystemTaskClass::StorageDaemon
            | SystemTaskClass::NetworkDaemon
            | SystemTaskClass::KernelThread
            | SystemTaskClass::IrqThread
    ) {
        return false;
    }

    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();
    comm == "systemd"
        || comm == "dbus-daemon"
        || comm == "networkmanager"
        || comm == "udisksd"
        || comm == "sshd"
        || comm.starts_with("systemd-")
        || cmdline.contains("/lib/systemd/")
        || cmdline.contains("/usr/lib/systemd/")
}

fn safety_warning_reason(warning: &SafetyWarning) -> String {
    match warning {
        SafetyWarning::CriticalRealtimePresent { pid, comm } => format!(
            "safety: critical realtime/input process present pid={} comm='{}'; never lower or deprioritize this task",
            pid, comm
        ),
        SafetyWarning::CompositorInFocusGroup { pid, comm } => format!(
            "safety: compositor process present pid={} comm='{}'; compositor is foreground latency context, not disposable background load",
            pid, comm
        ),
        SafetyWarning::UnknownForegroundLike { pid, comm } => format!(
            "safety: unknown active foreground-like process present pid={} comm='{}'; keep it Interactive/Unknown rather than Background",
            pid, comm
        ),
        SafetyWarning::TooBroadSystemServiceGroup { root_pids } => format!(
            "safety: broad service/system tree roots {:?}; do not select this as a mutation target",
            root_pids
        ),
    }
}

pub fn situation_for_group(group: &FocusGroup) -> SituationKind {
    match group.kind {
        FocusGroupKind::Game => SituationKind::GameFocused,
        FocusGroupKind::Browser => SituationKind::BrowserFocused,
        FocusGroupKind::Compile => SituationKind::CompileLoad,
        FocusGroupKind::Media => SituationKind::MediaPlayback,
        FocusGroupKind::Recording => SituationKind::Recording,
        FocusGroupKind::VirtualMachine => SituationKind::VirtualMachineLoad,
        FocusGroupKind::Idle => SituationKind::Idle,
        FocusGroupKind::Desktop | FocusGroupKind::Unknown => SituationKind::Unknown,
    }
}

pub fn focus_snapshot_at(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    foreground: Option<&ForegroundWindowSnapshot>,
) -> FocusSnapshot {
    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();
    let processes = crate::process_tree::scan_processes_at(
        proc_root,
        &mut cache.proc_cache,
        &budget,
        &mut budget_report,
    );

    build_focus_snapshot_from_processes(
        proc_root,
        cache,
        elapsed_ms,
        processes,
        foreground.cloned(),
    )
}

fn build_focus_snapshot_from_processes(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    processes: BTreeMap<u32, crate::process_tree::ProcInfo>,
    foreground: Option<ForegroundWindowSnapshot>,
) -> FocusSnapshot {
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }
    for children in children_by_parent.values_mut() {
        children.sort_unstable();
    }

    let mut focus_processes = BTreeMap::new();
    let mut next_counters = BTreeMap::new();

    for proc_info in processes.values() {
        let mut counters = read_focus_counters_at(proc_root, proc_info.pid);
        counters.starttime_ticks = proc_info.starttime_ticks.or(counters.starttime_ticks);

        let deltas = counter_deltas(cache.previous.get(&proc_info.pid), &counters);

        let exe_path = non_empty_str(&proc_info.exe_path);
        let cgroup_path = non_empty_str(&proc_info.cgroup_path);
        let is_foreground_window_process = foreground
            .as_ref()
            .and_then(|fg| fg.pid)
            .is_some_and(|pid| pid == proc_info.pid);

        let classification = classify_process(&ProcessIdentity {
            pid: proc_info.pid,
            ppid: proc_info.ppid,
            comm: &proc_info.comm,
            cmdline: &proc_info.cmdline,
            exe_path,
            cgroup_path,
            sched_policy: proc_info.sched_policy,
        });

        let focus_process = FocusProcess {
            pid: proc_info.pid,
            ppid: proc_info.ppid,
            comm: proc_info.comm.clone(),
            cmdline: proc_info.cmdline.clone(),
            cgroup_path: cgroup_path.map(PathBuf::from),
            starttime_ticks: proc_info.starttime_ticks.or(counters.starttime_ticks),
            sched_policy: proc_info.sched_policy,
            is_foreground_window_process,
            classification,
            cpu_time_ticks_delta: deltas.cpu_time_ticks,
            read_bytes_delta: deltas.read_bytes,
            write_bytes_delta: deltas.write_bytes,
            voluntary_ctxt_switches_delta: deltas.voluntary_ctxt_switches,
            nonvoluntary_ctxt_switches_delta: deltas.nonvoluntary_ctxt_switches,
        };

        next_counters.insert(proc_info.pid, counters);
        focus_processes.insert(proc_info.pid, focus_process);
    }

    cache.previous = next_counters;

    let mut snapshot = FocusSnapshot {
        elapsed_ms,
        foreground,
        processes: focus_processes,
        children_by_parent,
        groups: Vec::new(),
    };
    snapshot.groups = build_focus_groups(&snapshot);

    snapshot
}

fn non_empty_str(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn read_focus_counters_at(proc_root: &Path, pid: u32) -> FocusCounters {
    let mut counters = FocusCounters::default();
    let process_root = proc_root.join(pid.to_string());

    if let Some((cpu_time_ticks, starttime_ticks)) =
        read_focus_stat_counters_at(&process_root.join("stat"))
    {
        counters.cpu_time_ticks = cpu_time_ticks;
        counters.starttime_ticks = starttime_ticks;
    }

    if let Some((read_bytes, write_bytes)) = read_focus_io_counters_at(&process_root.join("io")) {
        counters.read_bytes = read_bytes;
        counters.write_bytes = write_bytes;
    }

    if let Some((voluntary, nonvoluntary)) =
        read_focus_status_context_switches_at(&process_root.join("status"))
    {
        counters.voluntary_ctxt_switches = voluntary;
        counters.nonvoluntary_ctxt_switches = nonvoluntary;
    }

    counters
}

fn read_focus_stat_counters_at(path: &Path) -> Option<(u64, Option<u64>)> {
    let stat = fs::read_to_string(path).ok()?;
    let (_, after_comm) = stat.rsplit_once(") ")?;
    let fields = after_comm.split_whitespace().collect::<Vec<_>>();

    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let starttime_ticks = fields.get(19).and_then(|value| value.parse::<u64>().ok());

    Some((utime.saturating_add(stime), starttime_ticks))
}

fn read_focus_io_counters_at(path: &Path) -> Option<(u64, u64)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut read_bytes = 0;
    let mut write_bytes = 0;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("read_bytes:") {
            read_bytes = value.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("write_bytes:") {
            write_bytes = value.trim().parse::<u64>().unwrap_or(0);
        }
    }

    Some((read_bytes, write_bytes))
}

fn read_focus_status_context_switches_at(path: &Path) -> Option<(u64, u64)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut voluntary = 0;
    let mut nonvoluntary = 0;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("voluntary_ctxt_switches:") {
            voluntary = value.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
            nonvoluntary = value.trim().parse::<u64>().unwrap_or(0);
        }
    }

    Some((voluntary, nonvoluntary))
}

fn counter_deltas(previous: Option<&FocusCounters>, current: &FocusCounters) -> FocusCounters {
    let Some(previous) = previous else {
        return FocusCounters {
            starttime_ticks: current.starttime_ticks,
            ..FocusCounters::default()
        };
    };

    if previous.starttime_ticks != current.starttime_ticks {
        return FocusCounters {
            starttime_ticks: current.starttime_ticks,
            ..FocusCounters::default()
        };
    }

    FocusCounters {
        starttime_ticks: current.starttime_ticks,
        cpu_time_ticks: current
            .cpu_time_ticks
            .saturating_sub(previous.cpu_time_ticks),
        read_bytes: current.read_bytes.saturating_sub(previous.read_bytes),
        write_bytes: current.write_bytes.saturating_sub(previous.write_bytes),
        voluntary_ctxt_switches: current
            .voluntary_ctxt_switches
            .saturating_sub(previous.voluntary_ctxt_switches),
        nonvoluntary_ctxt_switches: current
            .nonvoluntary_ctxt_switches
            .saturating_sub(previous.nonvoluntary_ctxt_switches),
    }
}

fn build_focus_groups(snapshot: &FocusSnapshot) -> Vec<FocusGroup> {
    let mut claimed_pids = BTreeSet::new();
    let mut groups = Vec::new();

    if let Some(group) = build_game_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_browser_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_compile_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in
        build_tree_groups_for_kind(snapshot, &claimed_pids, FocusGroupKind::Media, |process| {
            process.classification.class == SystemTaskClass::Media
        })
    {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::Recording,
        |process| process.classification.class == SystemTaskClass::Recorder,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::VirtualMachine,
        |process| process.classification.class == SystemTaskClass::VirtualMachine,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_desktop_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_idle_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_fallback_group(snapshot, &claimed_pids) {
        groups.push(group);
    }

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                priority_band_rank(right.priority_band).cmp(&priority_band_rank(left.priority_band))
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    groups
}

fn build_game_group(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Option<FocusGroup> {
    let game_like_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            is_game_class(process.classification.class) || is_game_runtime_process(process)
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if game_like_pids.is_empty() {
        return None;
    }

    let root_pid = game_like_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::GameScope)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| process.classification.class == SystemTaskClass::Game)
                        .unwrap_or(false)
                })
                .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &game_like_pids))
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| {
                            is_game_runtime_process(process)
                                && descendants_of_pid(snapshot, process.pid).iter().any(
                                    |child_pid| {
                                        snapshot
                                            .processes
                                            .get(child_pid)
                                            .map(|child| {
                                                child.classification.class == SystemTaskClass::Game
                                                    || child.classification.class
                                                        == SystemTaskClass::GameRenderThread
                                                    || child.classification.class
                                                        == SystemTaskClass::GameWorkerThread
                                            })
                                            .unwrap_or(false)
                                    },
                                )
                        })
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })?;

    let mut member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<BTreeSet<_>>();

    for process in snapshot.processes.values() {
        if claimed_pids.contains(&process.pid) {
            continue;
        }

        if process.classification.class == SystemTaskClass::WineServer
            && process_appears_tied_to_root(snapshot, process.pid, root_pid)
        {
            member_pids.insert(process.pid);
        }
    }

    for pid in &game_like_pids {
        if !claimed_pids.contains(pid) && process_appears_tied_to_root(snapshot, *pid, root_pid) {
            member_pids.insert(*pid);
        }
    }

    make_focus_group(
        snapshot,
        FocusGroupKind::Game,
        vec![root_pid],
        member_pids.into_iter().collect(),
        Some(root_pid),
        vec![
            "game group selected from gamescope/game/runtime roots".to_owned(),
            "wineserver is included only when tied to the same parent/session/cgroup/runtime"
                .to_owned(),
        ],
    )
}

fn build_browser_groups(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Vec<FocusGroup> {
    let browser_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_browser_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = browser_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| {
                    process.classification.class == SystemTaskClass::BrowserForeground
                        || !has_ancestor_in_set(snapshot, *pid, &browser_pids)
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| is_browser_class(process.classification.class))
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        member_pids.insert(root_pid);

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        process.classification.class == SystemTaskClass::BrowserForeground
                    })
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or(Some(root_pid));

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Browser,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "browser group rooted at browser parent process".to_owned(),
                "renderer/GPU/network descendants are included under the browser parent".to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_compile_groups(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Vec<FocusGroup> {
    let compile_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_compile_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if compile_pids.is_empty() {
        return Vec::new();
    }

    let mut roots = compile_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_stable_build_root)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter_map(|pid| nearest_compile_session_root(snapshot, pid))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &compile_pids))
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| {
        stable_build_root_rank(snapshot, *right)
            .cmp(&stable_build_root_rank(snapshot, *left))
            .then_with(|| compare_process_preference(snapshot, *right, *left))
    });

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) || claimed_pids.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        is_compile_class(process.classification.class)
                            || process.pid == root_pid
                            || process.classification.class == SystemTaskClass::Terminal
                            || process.classification.class == SystemTaskClass::Shell
                    })
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        if compile_pids.contains(&root_pid) {
            member_pids.insert(root_pid);
        }

        if !member_pids.iter().any(|pid| compile_pids.contains(pid)) {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(is_stable_build_root)
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or_else(|| {
                member_pids
                    .iter()
                    .copied()
                    .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            });

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Compile,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "compile group prefers stable build roots such as cargo/ninja/make/cmake/meson"
                    .to_owned(),
                "compiler/linker descendants are grouped under the stable build/session root"
                    .to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_tree_groups_for_kind<F>(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
    kind: FocusGroupKind,
    predicate: F,
) -> Vec<FocusGroup>
where
    F: Fn(&FocusProcess) -> bool,
{
    let matching_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| predicate(process))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = matching_pids
        .iter()
        .copied()
        .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &matching_pids))
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = matching_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| matching_pids.contains(pid) || *pid == root_pid)
            .collect::<Vec<_>>();

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

        if let Some(group) = make_focus_group(
            snapshot,
            kind,
            vec![root_pid],
            member_pids.clone(),
            primary_pid,
            vec![format!("{kind:?} group rooted at stable process tree")],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_desktop_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let desktop_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Desktop
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if desktop_pids.is_empty() {
        return None;
    }

    let primary_pid = desktop_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::Compositor)
                .unwrap_or(false)
        })
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_active_foreground_candidate)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(is_active_foreground_candidate)
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        });

    make_focus_group(
        snapshot,
        FocusGroupKind::Desktop,
        root_pids_from_members(snapshot, &desktop_pids),
        desktop_pids.into_iter().collect(),
        primary_pid,
        vec![
            "desktop group is supporting context".to_owned(),
            "compositor becomes primary only when it is an active foreground-latency candidate"
                .to_owned(),
        ],
    )
}

fn build_idle_group(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Option<FocusGroup> {
    let idle_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Idle
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if idle_pids.is_empty() {
        return None;
    }

    let primary_pid = idle_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Idle,
        root_pids_from_members(snapshot, &idle_pids),
        idle_pids.into_iter().collect(),
        primary_pid,
        vec!["idle/background service group is never allowed to outrank active foreground work by base score alone".to_owned()],
    )
}

fn build_fallback_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let root_pid = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_non_service_interactive_class(process.classification.class))
        .max_by(|left, right| {
            left.cpu_time_ticks_delta
                .cmp(&right.cpu_time_ticks_delta)
                .then_with(|| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|process| process.pid)?;

    let member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<Vec<_>>();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = member_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Unknown,
        vec![root_pid],
        member_pids,
        primary_pid,
        vec![
            "fallback selected highest non-service interactive process tree by recent CPU delta"
                .to_owned(),
        ],
    )
}

fn make_focus_group(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    mut root_pids: Vec<u32>,
    mut member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    mut reasons: Vec<String>,
) -> Option<FocusGroup> {
    root_pids.sort_unstable();
    root_pids.dedup();
    member_pids.sort_unstable();
    member_pids.dedup();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = primary_pid.or_else(|| {
        member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
    });

    let score_breakdown = score_focus_group(snapshot, kind, &root_pids, &member_pids, primary_pid);
    let score = clamp_score(
        score_breakdown.cpu_score
            + score_breakdown.io_score
            + score_breakdown.interactivity_score
            + score_breakdown.class_priority_score
            + score_breakdown.stability_score
            - score_breakdown.penalty,
    );

    let confidence = focus_group_confidence(snapshot, &member_pids, &score_breakdown);

    let priority_band = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.priority_band)
        .max_by_key(|band| priority_band_rank(*band))
        .unwrap_or(PriorityBand::Unknown);

    let display_name = display_name_for_group(
        kind,
        primary_pid.and_then(|pid| snapshot.processes.get(&pid)),
    );

    if let Some(primary_pid) = primary_pid
        && let Some(primary) = snapshot.processes.get(&primary_pid)
    {
        reasons.push(format!(
            "primary pid={} comm='{}' class={:?}",
            primary.pid, primary.comm, primary.classification.class
        ));
    }

    let mut group = FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name,
        score,
        score_breakdown,
        confidence,
        priority_band,
        reasons,
    };

    let safety_warnings = safety_warnings_for_group(&group, snapshot);
    group
        .reasons
        .extend(safety_warnings.iter().map(safety_warning_reason));

    Some(group)
}

fn root_pids_from_members(snapshot: &FocusSnapshot, member_pids: &BTreeSet<u32>) -> Vec<u32> {
    member_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| !member_pids.contains(&process.ppid))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
}

fn descendants_of_pid(snapshot: &FocusSnapshot, root_pid: u32) -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    let mut stack = vec![root_pid];

    while let Some(pid) = stack.pop() {
        if !result.insert(pid) {
            continue;
        }

        if let Some(children) = snapshot.children_by_parent.get(&pid) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    result
}

fn has_ancestor_in_set(snapshot: &FocusSnapshot, pid: u32, pids: &BTreeSet<u32>) -> bool {
    let mut current = pid;
    let mut seen = BTreeSet::new();

    while let Some(process) = snapshot.processes.get(&current) {
        if !seen.insert(current) {
            return false;
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            return false;
        }

        if pids.contains(&parent) {
            return true;
        }

        current = parent;
    }

    false
}

fn compare_process_preference(
    snapshot: &FocusSnapshot,
    left_pid: u32,
    right_pid: u32,
) -> std::cmp::Ordering {
    let left_score = snapshot
        .processes
        .get(&left_pid)
        .map(process_focus_score)
        .unwrap_or_default();
    let right_score = snapshot
        .processes
        .get(&right_pid)
        .map(process_focus_score)
        .unwrap_or_default();

    left_score
        .partial_cmp(&right_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left_pid.cmp(&right_pid))
}

fn process_appears_tied_to_root(snapshot: &FocusSnapshot, pid: u32, root_pid: u32) -> bool {
    if pid == root_pid {
        return true;
    }

    let Some(process) = snapshot.processes.get(&pid) else {
        return false;
    };
    let Some(root) = snapshot.processes.get(&root_pid) else {
        return false;
    };

    descendants_of_pid(snapshot, root_pid).contains(&pid)
        || process.ppid == root.ppid
        || same_non_empty_cgroup(process, root)
        || (contains_game_runtime_text(process) && contains_game_runtime_text(root))
}

fn same_non_empty_cgroup(left: &FocusProcess, right: &FocusProcess) -> bool {
    match (&left.cgroup_path, &right.cgroup_path) {
        (Some(left), Some(right)) => {
            !left.as_os_str().is_empty() && !right.as_os_str().is_empty() && left == right
        }
        _ => false,
    }
}

fn contains_game_runtime_text(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("steamapps")
        || text.contains("pressure-vessel")
        || text.contains("proton")
        || text.contains("wineserver")
}

fn is_game_runtime_process(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("pressure-vessel")
        || text.contains("steam-runtime")
        || text.contains("proton")
        || text.contains("steamapps")
}

fn is_game_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
    )
}

fn is_browser_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    )
}

fn is_compile_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BuildJob
            | SystemTaskClass::Compiler
            | SystemTaskClass::Linker
            | SystemTaskClass::Indexer
            | SystemTaskClass::PackageManager
    )
}

fn is_non_service_interactive_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::AudioRealtime
            | SystemTaskClass::Input
            | SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
            | SystemTaskClass::Compositor
            | SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
            | SystemTaskClass::Editor
            | SystemTaskClass::Terminal
            | SystemTaskClass::Shell
            | SystemTaskClass::Media
            | SystemTaskClass::Recorder
            | SystemTaskClass::VirtualMachine
    )
}

fn is_active_foreground_candidate(process: &FocusProcess) -> bool {
    process.cpu_time_ticks_delta > 0
        || process.read_bytes_delta > 0
        || process.write_bytes_delta > 0
        || process.voluntary_ctxt_switches_delta > 0
        || process.nonvoluntary_ctxt_switches_delta > 0
}

fn is_stable_build_root(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    matches!(
        comm.as_str(),
        "cargo" | "ninja" | "make" | "cmake" | "meson" | "scons"
    ) || process.classification.class == SystemTaskClass::BuildJob
}

fn stable_build_root_rank(snapshot: &FocusSnapshot, pid: u32) -> u8 {
    snapshot
        .processes
        .get(&pid)
        .map(|process| {
            if is_stable_build_root(process) {
                3
            } else if process.classification.class == SystemTaskClass::Terminal {
                2
            } else if process.classification.class == SystemTaskClass::Shell {
                1
            } else {
                0
            }
        })
        .unwrap_or(0)
}

fn nearest_compile_session_root(snapshot: &FocusSnapshot, pid: u32) -> Option<u32> {
    let mut current = pid;
    let mut nearest_shell_or_terminal = None;
    let process_cgroup = snapshot
        .processes
        .get(&pid)
        .and_then(|process| process.cgroup_path.clone());

    while let Some(process) = snapshot.processes.get(&current) {
        if is_stable_build_root(process) {
            return Some(process.pid);
        }

        if matches!(
            process.classification.class,
            SystemTaskClass::Terminal | SystemTaskClass::Shell
        ) {
            nearest_shell_or_terminal = Some(process.pid);
            break; // Stop at the NEAREST shell/terminal
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            break;
        }

        current = parent;
    }

    nearest_shell_or_terminal.or_else(|| {
        process_cgroup.and_then(|cgroup| {
            snapshot
                .processes
                .values()
                .filter(|process| process.cgroup_path.as_ref() == Some(&cgroup))
                .filter(|process| {
                    matches!(
                        process.classification.class,
                        SystemTaskClass::Terminal | SystemTaskClass::Shell
                    )
                })
                .max_by(|left, right| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|process| process.pid)
        })
    })
}

fn process_identity_text(process: &FocusProcess) -> String {
    let cgroup_path = process
        .cgroup_path
        .as_ref()
        .map(|path| path.to_string_lossy())
        .unwrap_or_default();

    format!(
        "{} {} {}",
        process.comm.to_ascii_lowercase(),
        process.cmdline.to_ascii_lowercase(),
        cgroup_path.to_ascii_lowercase()
    )
}

fn score_focus_group(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    member_pids: &[u32],
    primary_pid: Option<u32>,
) -> FocusScoreBreakdown {
    let cpu_score = focus_group_cpu_score(snapshot, member_pids);
    let io_score = focus_group_io_score(snapshot, kind, member_pids);
    let interactivity_score = focus_group_interactivity_score(snapshot, member_pids);
    let class_priority_score = focus_group_class_priority_score(snapshot, kind, member_pids);
    let stability_score = focus_group_stability_score(snapshot, kind, root_pids, primary_pid);
    let penalty = focus_group_penalty(snapshot, kind, root_pids, member_pids, primary_pid);

    FocusScoreBreakdown {
        cpu_score,
        io_score,
        interactivity_score,
        class_priority_score,
        stability_score,
        foreground_score: 0.0,
        penalty,
    }
}

fn focus_group_cpu_score(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let cpu_ticks = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.cpu_time_ticks_delta)
        .sum::<u64>();

    clamp_score(cpu_ticks as f32 / 250.0)
}

fn focus_group_io_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    member_pids: &[u32],
) -> f32 {
    let io_bytes = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| {
            process
                .read_bytes_delta
                .saturating_add(process.write_bytes_delta)
        })
        .sum::<u64>();

    let base = clamp_score(io_bytes as f32 / 67_108_864.0);

    if kind == FocusGroupKind::Compile {
        let linker_io_bytes = member_pids
            .iter()
            .filter_map(|pid| snapshot.processes.get(pid))
            .filter(|process| process.classification.class == SystemTaskClass::Linker)
            .map(|process| {
                process
                    .read_bytes_delta
                    .saturating_add(process.write_bytes_delta)
            })
            .sum::<u64>();

        clamp_score(base + (linker_io_bytes as f32 / 33_554_432.0).min(0.20))
    } else {
        base
    }
}

fn focus_group_interactivity_score(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let ctxt_switches = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| {
            process
                .voluntary_ctxt_switches_delta
                .saturating_add(process.nonvoluntary_ctxt_switches_delta)
        })
        .sum::<u64>();

    clamp_score(ctxt_switches as f32 / 250.0)
}

fn focus_group_class_priority_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    member_pids: &[u32],
) -> f32 {
    match kind {
        FocusGroupKind::Game => score_game_class_evidence(snapshot, member_pids),
        FocusGroupKind::Browser => score_browser_class_evidence(snapshot, member_pids),
        FocusGroupKind::Compile => score_compile_class_evidence(snapshot, member_pids),
        FocusGroupKind::Media => score_media_class_evidence(snapshot, member_pids),
        FocusGroupKind::Recording => score_recording_class_evidence(snapshot, member_pids),
        FocusGroupKind::VirtualMachine => {
            score_virtual_machine_class_evidence(snapshot, member_pids)
        }
        FocusGroupKind::Desktop => score_desktop_class_evidence(snapshot, member_pids),
        FocusGroupKind::Idle => score_idle_class_evidence(snapshot, member_pids),
        FocusGroupKind::Unknown => 0.05,
    }
}

fn score_game_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Game)
            .unwrap_or(false)
    }) {
        score += 0.25;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::GameScope)
            .unwrap_or(false)
    }) {
        score += 0.20;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::WineServer)
            .unwrap_or(false)
    }) {
        score += 0.10;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process.classification.class == SystemTaskClass::GameRenderThread
                    || process.classification.class == SystemTaskClass::GameWorkerThread
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(contains_game_runtime_text)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process_identity_text(process).contains("steamapps/common")
                    || process_identity_text(process).contains("steamapps")
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    clamp_score(score)
}

fn score_browser_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserForeground)
            .unwrap_or(false)
    }) {
        score += 0.30;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserRenderer)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserGpu)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserNetwork)
            .unwrap_or(false)
    }) {
        score += 0.10;
    }

    if active_process_count(snapshot, member_pids) >= 2 {
        score += 0.20;
    }

    clamp_score(score)
}

fn score_compile_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_stable_build_root)
            .unwrap_or(false)
    }) {
        score += 0.35;
    }

    let active_compiler_or_linker_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| {
            matches!(
                process.classification.class,
                SystemTaskClass::Compiler | SystemTaskClass::Linker
            ) && is_active_foreground_candidate(process)
        })
        .count();

    score += (active_compiler_or_linker_count as f32 * 0.12).min(0.35);

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process.classification.class == SystemTaskClass::Linker
                    && process
                        .read_bytes_delta
                        .saturating_add(process.write_bytes_delta)
                        > 0
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if total_cpu_ticks(snapshot, member_pids) >= 100 {
        score += 0.15;
    }

    clamp_score(score)
}

fn score_media_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_media = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Media)
            .unwrap_or(false)
    });

    if has_media {
        clamp_score(0.35 + low_to_moderate_activity_bonus(snapshot, member_pids))
    } else {
        0.0
    }
}

fn score_recording_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_recorder = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Recorder)
            .unwrap_or(false)
    });

    if has_recorder {
        clamp_score(0.40 + low_to_moderate_activity_bonus(snapshot, member_pids))
    } else {
        0.0
    }
}

fn score_virtual_machine_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_vm = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::VirtualMachine)
            .unwrap_or(false)
    });

    if has_vm {
        clamp_score(0.45 + focus_group_cpu_score(snapshot, member_pids) * 0.25)
    } else {
        0.0
    }
}

fn score_desktop_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Compositor)
            .unwrap_or(false)
    }) {
        score += 0.25;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                matches!(
                    process.classification.class,
                    SystemTaskClass::AudioRealtime
                        | SystemTaskClass::Input
                        | SystemTaskClass::Editor
                        | SystemTaskClass::Terminal
                        | SystemTaskClass::Shell
                )
            })
            .unwrap_or(false)
    }) {
        score += 0.20;
    }

    clamp_score(score)
}

fn score_idle_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_idle_class = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                matches!(
                    process.classification.class,
                    SystemTaskClass::Service
                        | SystemTaskClass::StorageDaemon
                        | SystemTaskClass::NetworkDaemon
                        | SystemTaskClass::KernelThread
                        | SystemTaskClass::IrqThread
                )
            })
            .unwrap_or(false)
    });

    if has_idle_class { 0.05 } else { 0.0 }
}

fn low_to_moderate_activity_bonus(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let cpu_ticks = total_cpu_ticks(snapshot, member_pids);
    if cpu_ticks == 0 {
        0.0
    } else if cpu_ticks <= 150 {
        0.25
    } else {
        0.15
    }
}

fn focus_group_stability_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    primary_pid: Option<u32>,
) -> f32 {
    let mut score = 0.0_f32;

    for root_pid in root_pids {
        if let Some(root) = snapshot.processes.get(root_pid) {
            match kind {
                FocusGroupKind::Game => {
                    if root.classification.class == SystemTaskClass::GameScope
                        || root.classification.class == SystemTaskClass::Game
                        || is_game_runtime_process(root)
                    {
                        score += 0.20;
                    }
                }
                FocusGroupKind::Browser => {
                    if root.classification.class == SystemTaskClass::BrowserForeground {
                        score += 0.25;
                    }
                }
                FocusGroupKind::Compile => {
                    if is_stable_build_root(root) {
                        score += 0.30;
                    } else if matches!(
                        root.classification.class,
                        SystemTaskClass::Terminal | SystemTaskClass::Shell
                    ) {
                        score += 0.15;
                    }
                }
                FocusGroupKind::Media
                | FocusGroupKind::Recording
                | FocusGroupKind::VirtualMachine => {
                    if Some(root.pid) == primary_pid {
                        score += 0.15;
                    }
                }
                FocusGroupKind::Desktop => {
                    if root.classification.class == SystemTaskClass::Compositor {
                        score += 0.10;
                    }
                }
                FocusGroupKind::Idle | FocusGroupKind::Unknown => {}
            }
        }
    }

    clamp_score(score)
}

fn focus_group_penalty(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    member_pids: &[u32],
    primary_pid: Option<u32>,
) -> f32 {
    match kind {
        FocusGroupKind::Game => game_group_penalty(snapshot, root_pids, member_pids),
        FocusGroupKind::Browser => browser_group_penalty(snapshot, member_pids),
        FocusGroupKind::Compile => compile_group_penalty(snapshot, member_pids),
        FocusGroupKind::Idle => idle_group_penalty(snapshot, member_pids),
        FocusGroupKind::Desktop => desktop_group_penalty(snapshot, primary_pid),
        FocusGroupKind::Media
        | FocusGroupKind::Recording
        | FocusGroupKind::VirtualMachine
        | FocusGroupKind::Unknown => 0.0,
    }
}

fn game_group_penalty(snapshot: &FocusSnapshot, root_pids: &[u32], member_pids: &[u32]) -> f32 {
    let root_is_launcher_only = root_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                let text = process_identity_text(process);
                text.contains("steam")
                    && !text.contains("steamapps")
                    && !text.contains("pressure-vessel")
                    && !text.contains("proton")
            })
            .unwrap_or(false)
    });

    let active_game_child_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| {
            process.classification.class == SystemTaskClass::Game
                || process.classification.class == SystemTaskClass::GameRenderThread
                || process.classification.class == SystemTaskClass::GameWorkerThread
        })
        .filter(|process| is_active_foreground_candidate(process))
        .count();

    if root_is_launcher_only && active_game_child_count == 0 {
        0.45
    } else if total_cpu_ticks(snapshot, member_pids) < 5 && active_game_child_count == 0 {
        0.20
    } else {
        0.0
    }
}

fn browser_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let idle_renderer_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| process.classification.class == SystemTaskClass::BrowserRenderer)
        .filter(|process| !is_active_foreground_candidate(process))
        .count();

    let active_child_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| process.classification.class != SystemTaskClass::BrowserForeground)
        .filter(|process| is_active_foreground_candidate(process))
        .count();

    if idle_renderer_count > active_child_count.saturating_mul(2).saturating_add(2) {
        ((idle_renderer_count - active_child_count) as f32 * 0.04).min(0.25)
    } else {
        0.0
    }
}

fn compile_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_stable_build_root = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_stable_build_root)
            .unwrap_or(false)
    });

    let active_compiler_or_linker_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| {
            matches!(
                process.classification.class,
                SystemTaskClass::Compiler | SystemTaskClass::Linker
            ) && is_active_foreground_candidate(process)
        })
        .count();

    let indexer_only = member_pids.iter().all(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Indexer)
            .unwrap_or(false)
    });

    if indexer_only {
        0.55
    } else if !has_stable_build_root && active_compiler_or_linker_count == 0 {
        0.35
    } else {
        0.0
    }
}

fn idle_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    if total_cpu_ticks(snapshot, member_pids) == 0 {
        0.20
    } else {
        0.10
    }
}

fn desktop_group_penalty(snapshot: &FocusSnapshot, primary_pid: Option<u32>) -> f32 {
    let Some(primary_pid) = primary_pid else {
        return 0.10;
    };

    let Some(primary) = snapshot.processes.get(&primary_pid) else {
        return 0.10;
    };

    if primary.classification.class == SystemTaskClass::Compositor
        && !is_active_foreground_candidate(primary)
    {
        0.20
    } else {
        0.0
    }
}

fn focus_group_confidence(
    snapshot: &FocusSnapshot,
    member_pids: &[u32],
    breakdown: &FocusScoreBreakdown,
) -> f32 {
    let max_class_confidence = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.confidence)
        .fold(0.0_f32, f32::max);

    let activity_evidence =
        clamp_score(breakdown.cpu_score + breakdown.io_score + breakdown.interactivity_score);

    let mut confidence = clamp_score(
        (max_class_confidence * 0.45)
            + (activity_evidence * 0.40)
            + (breakdown.stability_score * 0.25)
            - (breakdown.penalty * 0.50),
    );

    if activity_evidence < 0.05 {
        confidence = confidence.min(0.55);
    } else if activity_evidence < 0.20 {
        confidence = confidence.min(0.75);
    }

    confidence
}

fn active_process_count(snapshot: &FocusSnapshot, member_pids: &[u32]) -> usize {
    member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| is_active_foreground_candidate(process))
        .count()
}

fn total_cpu_ticks(snapshot: &FocusSnapshot, member_pids: &[u32]) -> u64 {
    member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.cpu_time_ticks_delta)
        .sum::<u64>()
}

fn clamp_score(score: f32) -> f32 {
    score.clamp(0.0, 1.0)
}

fn focus_group_kind_for_class(class: SystemTaskClass) -> FocusGroupKind {
    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => FocusGroupKind::Game,

        SystemTaskClass::BrowserForeground
        | SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork => FocusGroupKind::Browser,

        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => FocusGroupKind::Compile,

        SystemTaskClass::Media => FocusGroupKind::Media,
        SystemTaskClass::Recorder => FocusGroupKind::Recording,
        SystemTaskClass::VirtualMachine => FocusGroupKind::VirtualMachine,

        SystemTaskClass::Compositor | SystemTaskClass::AudioRealtime | SystemTaskClass::Input => {
            FocusGroupKind::Desktop
        }

        SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Unknown => FocusGroupKind::Unknown,

        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service => FocusGroupKind::Idle,
    }
}

fn process_focus_score(process: &FocusProcess) -> f32 {
    let class_base = match process.classification.class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input => 90.0,
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => 80.0,
        SystemTaskClass::Compositor | SystemTaskClass::BrowserForeground => 70.0,
        SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => 50.0,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => 35.0,
        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service
        | SystemTaskClass::BrowserBackground => 15.0,
        SystemTaskClass::Unknown => 0.0,
    };

    let cpu_score = process.cpu_time_ticks_delta as f32;
    let io_score = (process
        .read_bytes_delta
        .saturating_add(process.write_bytes_delta) as f32)
        / 1_048_576.0;
    let ctxt_score = (process
        .voluntary_ctxt_switches_delta
        .saturating_add(process.nonvoluntary_ctxt_switches_delta) as f32)
        * 0.05;

    class_base + process.classification.confidence + cpu_score + io_score + ctxt_score
}

fn display_name_for_group(kind: FocusGroupKind, primary: Option<&FocusProcess>) -> String {
    if let Some(primary) = primary
        && !primary.comm.is_empty()
    {
        return primary.comm.clone();
    }

    match kind {
        FocusGroupKind::Game => "Game".to_owned(),
        FocusGroupKind::Browser => "Browser".to_owned(),
        FocusGroupKind::Compile => "Compile".to_owned(),
        FocusGroupKind::Media => "Media".to_owned(),
        FocusGroupKind::Recording => "Recording".to_owned(),
        FocusGroupKind::VirtualMachine => "VirtualMachine".to_owned(),
        FocusGroupKind::Desktop => "Desktop".to_owned(),
        FocusGroupKind::Idle => "Idle".to_owned(),
        FocusGroupKind::Unknown => "Unknown".to_owned(),
    }
}

fn priority_band_rank(priority_band: PriorityBand) -> u8 {
    match priority_band {
        PriorityBand::Unknown => 0,
        PriorityBand::Background => 1,
        PriorityBand::Throughput => 2,
        PriorityBand::Interactive => 3,
        PriorityBand::ForegroundLatency => 4,
        PriorityBand::CriticalRealtime => 5,
    }
}

pub fn classify_process(identity: &ProcessIdentity<'_>) -> Classification {
    let comm = identity.comm.to_ascii_lowercase();
    let cmdline = identity.cmdline.to_ascii_lowercase();
    let exe_path = identity.exe_path.unwrap_or_default().to_ascii_lowercase();
    let cgroup_path = identity
        .cgroup_path
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut reasons = Vec::new();

    let (class, confidence) = if comm.starts_with("irq/")
        || comm.starts_with("irq-")
        || comm.contains("irq")
    {
        reasons.push(format!("comm '{}' looks like an IRQ thread", identity.comm));
        (SystemTaskClass::IrqThread, 0.95)
    } else if identity.ppid == 2 || comm.starts_with("kworker") || comm.starts_with("ksoftirqd") {
        reasons.push(format!(
            "pid={} ppid={} comm='{}' looks like a kernel thread",
            identity.pid, identity.ppid, identity.comm
        ));
        (SystemTaskClass::KernelThread, 0.9)
    } else if comm == "wineserver" {
        reasons.push("comm is exactly 'wineserver'".to_owned());
        (SystemTaskClass::WineServer, 0.98)
    } else if comm == "gamescope"
        || exe_path.contains("/gamescope")
        || cgroup_path.contains("gamescope")
    {
        reasons.push("process identity matches gamescope".to_owned());
        (SystemTaskClass::GameScope, 0.95)
    } else if comm.contains("pipewire")
        || comm.contains("wireplumber")
        || comm.contains("pulseaudio")
        || comm.contains("jackd")
    {
        reasons.push(format!("comm '{}' matches an audio service", identity.comm));
        (SystemTaskClass::AudioRealtime, 0.9)
    } else if comm.contains("sway")
        || comm.contains("kwin")
        || comm.contains("mutter")
        || comm.contains("gnome-shell")
        || comm.contains("weston")
        || comm.contains("hyprland")
    {
        reasons.push(format!("comm '{}' matches a compositor", identity.comm));
        (SystemTaskClass::Compositor, 0.9)
    } else if comm.contains("firefox")
        || comm.contains("chrome")
        || comm.contains("chromium")
        || comm.contains("brave")
        || comm.contains("browser")
        || exe_path.contains("/firefox")
        || exe_path.contains("/chrome")
        || exe_path.contains("/chromium")
    {
        reasons.push("process identity matches a browser family".to_owned());
        if cmdline.contains("--type=gpu-process") {
            reasons.push("cmdline contains '--type=gpu-process'".to_owned());
            (SystemTaskClass::BrowserGpu, 0.9)
        } else if cmdline.contains("--type=renderer") || cmdline.contains("web content") {
            reasons.push("cmdline indicates a browser renderer child".to_owned());
            (SystemTaskClass::BrowserRenderer, 0.85)
        } else if cmdline.contains("--type=utility") && cmdline.contains("network") {
            reasons.push("cmdline indicates a browser network utility child".to_owned());
            (SystemTaskClass::BrowserNetwork, 0.85)
        } else if cgroup_path.contains("background") {
            reasons.push("cgroup path contains 'background'".to_owned());
            (SystemTaskClass::BrowserBackground, 0.75)
        } else {
            reasons.push("browser process role is foreground or parent by default".to_owned());
            (SystemTaskClass::BrowserForeground, 0.7)
        }
    } else if comm == "clang"
        || comm == "clang++"
        || comm == "gcc"
        || comm == "g++"
        || comm == "rustc"
        || comm == "cc1"
        || comm == "cc1plus"
    {
        reasons.push(format!("comm '{}' matches a compiler", identity.comm));
        (SystemTaskClass::Compiler, 0.95)
    } else if comm == "ld" || comm == "ld.lld" || comm == "mold" || comm == "gold" || comm == "lld"
    {
        reasons.push(format!("comm '{}' matches a linker", identity.comm));
        (SystemTaskClass::Linker, 0.95)
    } else if comm == "clangd"
        || comm.contains("rust-analyzer")
        || comm.contains("ccls")
        || comm.contains("indexer")
    {
        reasons.push(format!(
            "comm '{}' matches an indexer/language server",
            identity.comm
        ));
        (SystemTaskClass::Indexer, 0.9)
    } else if comm == "cargo"
        || comm == "make"
        || comm == "ninja"
        || comm == "cmake"
        || comm == "meson"
        || cmdline.contains(" emerge ")
        || comm == "emerge"
        || comm == "portage"
        || comm == "pacman"
        || comm == "apt"
        || comm == "dnf"
    {
        reasons.push(format!(
            "comm '{}' or cmdline matches build/package work",
            identity.comm
        ));
        if comm == "emerge"
            || comm == "portage"
            || comm == "pacman"
            || comm == "apt"
            || comm == "dnf"
        {
            (SystemTaskClass::PackageManager, 0.9)
        } else {
            (SystemTaskClass::BuildJob, 0.85)
        }
    } else if comm.contains("udisks")
        || comm.contains("jbd2")
        || comm.contains("btrfs")
        || comm.contains("zfs")
        || comm.contains("io_uring")
    {
        reasons.push(format!(
            "comm '{}' matches storage daemon/kernel storage work",
            identity.comm
        ));
        (SystemTaskClass::StorageDaemon, 0.8)
    } else if comm.contains("networkmanager")
        || comm.contains("systemd-network")
        || comm.contains("dhcpcd")
        || comm.contains("wpa_supplicant")
    {
        reasons.push(format!(
            "comm '{}' matches network daemon work",
            identity.comm
        ));
        (SystemTaskClass::NetworkDaemon, 0.85)
    } else if comm.contains("code")
        || comm.contains("vscodium")
        || comm.contains("kate")
        || comm.contains("nvim")
        || comm.contains("vim")
        || comm.contains("emacs")
    {
        reasons.push(format!("comm '{}' matches an editor", identity.comm));
        (SystemTaskClass::Editor, 0.8)
    } else if comm.contains("alacritty")
        || comm.contains("kitty")
        || comm.contains("wezterm")
        || comm.contains("foot")
        || comm.contains("gnome-terminal")
        || comm.contains("konsole")
    {
        reasons.push(format!("comm '{}' matches a terminal", identity.comm));
        (SystemTaskClass::Terminal, 0.85)
    } else if comm == "bash" || comm == "zsh" || comm == "fish" || comm == "sh" {
        reasons.push(format!("comm '{}' matches a shell", identity.comm));
        (SystemTaskClass::Shell, 0.9)
    } else if comm.contains("vlc")
        || comm.contains("mpv")
        || comm.contains("spotify")
        || comm.contains("pipewire-media-session")
    {
        reasons.push(format!("comm '{}' matches media playback", identity.comm));
        (SystemTaskClass::Media, 0.8)
    } else if comm.contains("obs")
        || comm.contains("gpu-screen-recorder")
        || comm.contains("recorder")
    {
        reasons.push(format!(
            "comm '{}' matches recording software",
            identity.comm
        ));
        (SystemTaskClass::Recorder, 0.85)
    } else if comm.contains("qemu")
        || comm.contains("virt")
        || comm.contains("virtualbox")
        || comm.contains("vmware")
    {
        reasons.push(format!(
            "comm '{}' matches virtual machine software",
            identity.comm
        ));
        (SystemTaskClass::VirtualMachine, 0.85)
    } else if identity.pid != 1
        && !cgroup_path.contains(".service")
        && !cgroup_path.contains("/system.slice/")
        && let Some(hit) = crate::community_rules::classify_process_identity(
            &crate::community_rules::CommunityProcessIdentity {
                thread_comm: identity.comm,
                process_comm: identity.comm,
                cmdline: identity.cmdline,
                exe_path: identity.exe_path.unwrap_or_default(),
                cgroup_path: identity.cgroup_path.unwrap_or_default(),
            },
        )
        && let Some(class) = system_class_for_community_task_class(hit.class)
    {
        reasons.push(hit.reason);
        (class, hit.confidence)
    } else if cgroup_path.contains("steam")
        || cgroup_path.contains("games")
        || cmdline.contains("steamapps")
        || exe_path.contains("steamapps")
    {
        reasons.push("cgroup, cmdline, or exe path suggests a game process".to_owned());
        (SystemTaskClass::Game, 0.75)
    } else if comm == "steam" || comm.contains("electron") || comm == "python" {
        reasons.push(format!(
            "comm '{}' is ambiguous and needs more evidence before assigning a specific class",
            identity.comm
        ));
        (SystemTaskClass::Unknown, 0.35)
    } else if identity.pid == 1
        || cgroup_path.contains(".service")
        || cgroup_path.contains("/system.slice/")
    {
        reasons.push("pid/cgroup suggests a generic service".to_owned());
        (SystemTaskClass::Service, 0.6)
    } else {
        reasons.push("no strong process classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

fn system_class_for_community_task_class(
    class: crate::process_tree::TaskClass,
) -> Option<SystemTaskClass> {
    match class {
        crate::process_tree::TaskClass::Game => Some(SystemTaskClass::Game),
        _ => None,
    }
}

pub fn classify_thread(identity: &ThreadIdentity<'_>) -> Classification {
    let thread_comm = identity.thread_comm.to_ascii_lowercase();
    let process_comm = identity.process_comm.to_ascii_lowercase();
    let mut reasons = Vec::new();

    let (class, confidence) = if thread_comm.starts_with("irq/")
        || thread_comm.starts_with("irq-")
        || thread_comm.contains("irq")
    {
        reasons.push(format!(
            "thread_comm '{}' looks like an IRQ thread",
            identity.thread_comm
        ));
        (SystemTaskClass::IrqThread, 0.95)
    } else if thread_comm.contains("audio")
        || thread_comm.contains("pipewire")
        || thread_comm.contains("jack")
        || is_realtime_policy(identity.sched_policy)
    {
        reasons.push(format!(
            "thread_comm '{}' or sched_policy {:?} suggests realtime/audio work",
            identity.thread_comm, identity.sched_policy
        ));
        (SystemTaskClass::AudioRealtime, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("render")
        || thread_comm.contains("rhi")
        || thread_comm.contains("dxvk")
        || thread_comm.contains("vulkan")
        || thread_comm.contains("gpu"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests render/GPU work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameRenderThread, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("worker")
        || thread_comm.contains("task")
        || thread_comm.contains("job")
        || thread_comm.contains("pool"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests worker/job work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameWorkerThread, 0.8)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    ) && (thread_comm.contains("compositor") || thread_comm.contains("render"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests renderer work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserRenderer, 0.75)
    } else if matches!(identity.process_class, SystemTaskClass::BrowserForeground)
        && (thread_comm.contains("socket")
            || thread_comm.contains("network")
            || thread_comm.contains("dns"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests network work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserNetwork, 0.75)
    } else if identity.process_class != SystemTaskClass::Unknown {
        reasons.push(format!(
            "thread inherits process_class {:?} from process_comm '{}'",
            identity.process_class, identity.process_comm
        ));
        (identity.process_class, 0.6)
    } else if process_comm == "python"
        || process_comm.contains("electron")
        || process_comm == "steam"
    {
        reasons.push(format!(
            "process_comm '{}' is ambiguous and thread_comm '{}' did not disambiguate it",
            identity.process_comm, identity.thread_comm
        ));
        (SystemTaskClass::Unknown, 0.3)
    } else {
        reasons.push("no strong thread classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

pub fn priority_band_for_class(class: SystemTaskClass, sched_policy: Option<u32>) -> PriorityBand {
    if is_realtime_policy(sched_policy) {
        return PriorityBand::CriticalRealtime;
    }

    match class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input | SystemTaskClass::IrqThread => {
            PriorityBand::CriticalRealtime
        }
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameScope
        | SystemTaskClass::Compositor
        | SystemTaskClass::BrowserForeground => PriorityBand::ForegroundLatency,
        SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => PriorityBand::Interactive,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::PackageManager => PriorityBand::Throughput,
        SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Indexer
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::Service => PriorityBand::Background,
        SystemTaskClass::Unknown => PriorityBand::Unknown,
    }
}

pub fn legacy_task_class_for_system_class(
    class: SystemTaskClass,
) -> crate::process_tree::TaskClass {
    use crate::process_tree::TaskClass;

    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread => TaskClass::Game,

        SystemTaskClass::WineServer => TaskClass::WineServer,
        SystemTaskClass::GameScope => TaskClass::GameScope,
        SystemTaskClass::Compositor => TaskClass::Compositor,

        SystemTaskClass::Service
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread => TaskClass::Service,

        _ => TaskClass::Helper,
    }
}

fn is_realtime_policy(sched_policy: Option<u32>) -> bool {
    matches!(sched_policy, Some(SCHED_FIFO | SCHED_RR | SCHED_DEADLINE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    #[derive(Debug, Clone)]
    struct FakeProcProcess {
        pid: u32,
        ppid: u32,
        comm: String,
        cmdline: String,
        cgroup_path: String,
        sched_policy: Option<u32>,
        starttime_ticks: u64,
        cpu_time_ticks: u64,
        read_bytes: u64,
        write_bytes: u64,
        voluntary_ctxt_switches: u64,
        nonvoluntary_ctxt_switches: u64,
    }

    impl FakeProcProcess {
        fn new(pid: u32, ppid: u32, comm: &str, cmdline: &str) -> Self {
            Self {
                pid,
                ppid,
                comm: comm.to_owned(),
                cmdline: cmdline.to_owned(),
                cgroup_path: format!("/user.slice/test-{}.scope", pid),
                sched_policy: None,
                starttime_ticks: pid as u64 * 100,
                cpu_time_ticks: 0,
                read_bytes: 0,
                write_bytes: 0,
                voluntary_ctxt_switches: 0,
                nonvoluntary_ctxt_switches: 0,
            }
        }

        fn with_cgroup(mut self, cgroup_path: &str) -> Self {
            self.cgroup_path = cgroup_path.to_owned();
            self
        }

        fn with_sched_policy(mut self, sched_policy: u32) -> Self {
            self.sched_policy = Some(sched_policy);
            self
        }

        fn with_activity(
            mut self,
            cpu_time_ticks: u64,
            read_bytes: u64,
            write_bytes: u64,
            voluntary_ctxt_switches: u64,
            nonvoluntary_ctxt_switches: u64,
        ) -> Self {
            self.cpu_time_ticks = cpu_time_ticks;
            self.read_bytes = read_bytes;
            self.write_bytes = write_bytes;
            self.voluntary_ctxt_switches = voluntary_ctxt_switches;
            self.nonvoluntary_ctxt_switches = nonvoluntary_ctxt_switches;
            self
        }
    }

    fn focus_temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-focus-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_proc_process(proc_root: &Path, process: &FakeProcProcess) {
        let process_dir = proc_root.join(process.pid.to_string());
        std::fs::create_dir_all(&process_dir).unwrap();

        std::fs::write(
            process_dir.join("status"),
            format!(
                "Name:\t{}\nPPid:\t{}\nvoluntary_ctxt_switches:\t{}\nnonvoluntary_ctxt_switches:\t{}\n",
                process.comm,
                process.ppid,
                process.voluntary_ctxt_switches,
                process.nonvoluntary_ctxt_switches
            ),
        )
        .unwrap();

        std::fs::write(
            process_dir.join("cmdline"),
            process
                .cmdline
                .split(' ')
                .collect::<Vec<_>>()
                .join("\0")
                .into_bytes(),
        )
        .unwrap();

        std::fs::write(
            process_dir.join("cgroup"),
            format!("0::{}\n", process.cgroup_path),
        )
        .unwrap();

        std::fs::write(
            process_dir.join("io"),
            format!(
                "read_bytes: {}\nwrite_bytes: {}\n",
                process.read_bytes, process.write_bytes
            ),
        )
        .unwrap();

        std::fs::write(process_dir.join("stat"), fake_stat_line(process)).unwrap();
    }

    fn fake_stat_line(process: &FakeProcProcess) -> String {
        let mut fields_after_comm = vec![
            "S".to_owned(),
            process.ppid.to_string(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            process.cpu_time_ticks.to_string(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "20".to_owned(),
            "0".to_owned(),
            "1".to_owned(),
            "0".to_owned(),
            process.starttime_ticks.to_string(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            "0".to_owned(),
            process.sched_policy.unwrap_or(0).to_string(),
        ];

        while fields_after_comm.len() < 42 {
            fields_after_comm.push("0".to_owned());
        }

        format!(
            "{} ({}) {}",
            process.pid,
            process.comm,
            fields_after_comm.join(" ")
        )
    }

    fn focus_snapshot_from_fake_proc(
        test_name: &str,
        first_sample: Vec<FakeProcProcess>,
        second_sample: Vec<FakeProcProcess>,
    ) -> FocusSnapshot {
        let proc_root = focus_temp_dir(test_name);
        let mut cache = FocusCache::default();

        for process in &first_sample {
            write_fake_proc_process(&proc_root, process);
        }

        let _ = focus_snapshot_at(&proc_root, &mut cache, 0, None);

        for process in &second_sample {
            write_fake_proc_process(&proc_root, process);
        }

        let snapshot = focus_snapshot_at(&proc_root, &mut cache, 1000, None);
        std::fs::remove_dir_all(proc_root).ok();
        snapshot
    }

    fn required_test_classification(
        class: SystemTaskClass,
        priority_band: PriorityBand,
        confidence: f32,
    ) -> Classification {
        Classification {
            class,
            priority_band,
            confidence,
            reasons: vec![format!("required focus test classification {class:?}")],
        }
    }

    fn required_test_process(
        pid: u32,
        ppid: u32,
        comm: &str,
        class: SystemTaskClass,
        priority_band: PriorityBand,
        cpu_time_ticks_delta: u64,
    ) -> FocusProcess {
        FocusProcess {
            pid,
            ppid,
            comm: comm.to_owned(),
            cmdline: comm.to_owned(),
            cgroup_path: None,
            starttime_ticks: Some(pid as u64 * 100),
            sched_policy: None,
            is_foreground_window_process: false,
            classification: required_test_classification(class, priority_band, 0.85),
            cpu_time_ticks_delta,
            read_bytes_delta: 0,
            write_bytes_delta: 0,
            voluntary_ctxt_switches_delta: 0,
            nonvoluntary_ctxt_switches_delta: 0,
        }
    }

    fn required_test_group(
        kind: FocusGroupKind,
        root_pids: Vec<u32>,
        member_pids: Vec<u32>,
        primary_pid: Option<u32>,
        score: f32,
        confidence: f32,
    ) -> FocusGroup {
        FocusGroup {
            kind,
            root_pids,
            member_pids,
            primary_pid,
            display_name: format!("{kind:?}"),
            score,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence,
            priority_band: match kind {
                FocusGroupKind::Game | FocusGroupKind::Browser | FocusGroupKind::Desktop => {
                    PriorityBand::ForegroundLatency
                }
                FocusGroupKind::Compile => PriorityBand::Throughput,
                FocusGroupKind::Idle => PriorityBand::Background,
                FocusGroupKind::Media
                | FocusGroupKind::Recording
                | FocusGroupKind::VirtualMachine
                | FocusGroupKind::Unknown => PriorityBand::Interactive,
            },
            reasons: vec![format!("required focus test group {kind:?}")],
        }
    }

    fn required_test_snapshot(
        groups: Vec<FocusGroup>,
        processes: Vec<FocusProcess>,
        elapsed_ms: u64,
    ) -> FocusSnapshot {
        let mut process_map = BTreeMap::new();
        let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for process in processes {
            children_by_parent
                .entry(process.ppid)
                .or_default()
                .push(process.pid);
            process_map.insert(process.pid, process);
        }

        for children in children_by_parent.values_mut() {
            children.sort_unstable();
        }

        FocusSnapshot {
            elapsed_ms,
            foreground: None,
            processes: process_map,
            children_by_parent,
            groups,
        }
    }

    fn required_focus_policy() -> FocusPolicy {
        FocusPolicy {
            poll_ms: 1000,
            min_confidence: 0.60,
            switch_margin: 0.20,
            switch_cooldown_ms: 0,
            required_winner_polls: 2,
            max_roots: 4,
        }
    }

    fn first_focus_group(snapshot: &FocusSnapshot, kind: FocusGroupKind) -> &FocusGroup {
        snapshot
            .groups
            .iter()
            .find(|group| group.kind == kind)
            .unwrap()
    }

    #[test]
    fn safety_warnings_report_critical_realtime_and_compositor_members() {
        let audio = test_process(
            700,
            1,
            "pipewire",
            SystemTaskClass::AudioRealtime,
            PriorityBand::CriticalRealtime,
            5,
        );
        let compositor = test_process(
            701,
            1,
            "sway",
            SystemTaskClass::Compositor,
            PriorityBand::ForegroundLatency,
            10,
        );

        let snapshot = test_snapshot(vec![audio, compositor]);
        let group = FocusGroup {
            kind: FocusGroupKind::Desktop,
            root_pids: vec![700],
            member_pids: vec![700, 701],
            primary_pid: Some(701),
            display_name: "Desktop".to_owned(),
            score: 0.75,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.80,
            priority_band: PriorityBand::ForegroundLatency,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::CriticalRealtimePresent { pid: 700, comm } if comm == "pipewire"
        )));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::CompositorInFocusGroup { pid: 701, comm } if comm == "sway"
        )));
    }

    #[test]
    fn safety_warnings_report_unknown_active_foreground_like_process() {
        let mut unknown = test_process(
            710,
            1,
            "unknown-app",
            SystemTaskClass::Unknown,
            PriorityBand::Interactive,
            20,
        );
        unknown.voluntary_ctxt_switches_delta = 4;

        let snapshot = test_snapshot(vec![unknown]);
        let group = FocusGroup {
            kind: FocusGroupKind::Unknown,
            root_pids: vec![710],
            member_pids: vec![710],
            primary_pid: Some(710),
            display_name: "unknown-app".to_owned(),
            score: 0.50,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.50,
            priority_band: PriorityBand::Interactive,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::UnknownForegroundLike { pid: 710, comm } if comm == "unknown-app"
        )));
    }

    #[test]
    fn safety_warnings_report_broad_system_service_group() {
        let systemd = test_process(
            720,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        let dbus = test_process(
            721,
            720,
            "dbus-daemon",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        let network = test_process(
            722,
            720,
            "NetworkManager",
            SystemTaskClass::NetworkDaemon,
            PriorityBand::Background,
            0,
        );
        let storage = test_process(
            723,
            720,
            "udisksd",
            SystemTaskClass::StorageDaemon,
            PriorityBand::Background,
            0,
        );

        let snapshot = test_snapshot(vec![systemd, dbus, network, storage]);
        let group = FocusGroup {
            kind: FocusGroupKind::Idle,
            root_pids: vec![720],
            member_pids: vec![720, 721, 722, 723],
            primary_pid: Some(720),
            display_name: "systemd".to_owned(),
            score: 0.10,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.40,
            priority_band: PriorityBand::Background,
            reasons: Vec::new(),
        };

        let warnings = safety_warnings_for_group(&group, &snapshot);

        assert!(warnings.iter().any(|warning| matches!(
            warning,
            SafetyWarning::TooBroadSystemServiceGroup { root_pids } if root_pids == &vec![720]
        )));
    }

    #[test]
    fn make_focus_group_appends_safety_warning_reasons() {
        let audio = test_process(
            730,
            1,
            "pipewire",
            SystemTaskClass::AudioRealtime,
            PriorityBand::CriticalRealtime,
            5,
        );

        let snapshot = test_snapshot(vec![audio]);
        let group = make_focus_group(
            &snapshot,
            FocusGroupKind::Desktop,
            vec![730],
            vec![730],
            Some(730),
            vec!["desktop group test".to_owned()],
        )
        .unwrap();

        assert!(
            group
                .reasons
                .iter()
                .any(|reason| reason.contains("safety: critical realtime/input process present"))
        );
    }

    fn situation_mapping_test_group(kind: FocusGroupKind) -> FocusGroup {
        FocusGroup {
            kind,
            root_pids: vec![1],
            member_pids: vec![1],
            primary_pid: Some(1),
            display_name: format!("{kind:?}"),
            score: 0.75,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence: 0.75,
            priority_band: PriorityBand::Interactive,
            reasons: vec![format!("situation mapping test {kind:?}")],
        }
    }

    #[test]
    fn maps_focus_groups_to_autotune_situations() {
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Game)),
            SituationKind::GameFocused
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Browser)),
            SituationKind::BrowserFocused
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Compile)),
            SituationKind::CompileLoad
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Media)),
            SituationKind::MediaPlayback
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Recording)),
            SituationKind::Recording
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(
                FocusGroupKind::VirtualMachine
            )),
            SituationKind::VirtualMachineLoad
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Idle)),
            SituationKind::Idle
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Desktop)),
            SituationKind::Unknown
        );
        assert_eq!(
            situation_for_group(&situation_mapping_test_group(FocusGroupKind::Unknown)),
            SituationKind::Unknown
        );
    }

    fn resolver_test_classification(
        class: SystemTaskClass,
        priority_band: PriorityBand,
        confidence: f32,
    ) -> Classification {
        Classification {
            class,
            priority_band,
            confidence,
            reasons: vec![format!("resolver test classification {class:?}")],
        }
    }

    fn resolver_test_process(pid: u32, comm: &str, class: SystemTaskClass) -> FocusProcess {
        FocusProcess {
            pid,
            ppid: 1,
            comm: comm.to_owned(),
            cmdline: comm.to_owned(),
            cgroup_path: None,
            starttime_ticks: Some(pid as u64 * 100),
            sched_policy: None,
            is_foreground_window_process: false,
            classification: resolver_test_classification(class, PriorityBand::Interactive, 0.80),
            cpu_time_ticks_delta: 10,
            read_bytes_delta: 0,
            write_bytes_delta: 0,
            voluntary_ctxt_switches_delta: 0,
            nonvoluntary_ctxt_switches_delta: 0,
        }
    }

    fn resolver_test_group(
        kind: FocusGroupKind,
        root_pids: Vec<u32>,
        member_pids: Vec<u32>,
        primary_pid: Option<u32>,
        score: f32,
        confidence: f32,
    ) -> FocusGroup {
        FocusGroup {
            kind,
            root_pids,
            member_pids,
            primary_pid,
            display_name: format!("{kind:?}"),
            score,
            score_breakdown: FocusScoreBreakdown::default(),
            confidence,
            priority_band: match kind {
                FocusGroupKind::Game | FocusGroupKind::Browser | FocusGroupKind::Desktop => {
                    PriorityBand::ForegroundLatency
                }
                FocusGroupKind::Compile => PriorityBand::Throughput,
                FocusGroupKind::Idle => PriorityBand::Background,
                FocusGroupKind::Media
                | FocusGroupKind::Recording
                | FocusGroupKind::VirtualMachine
                | FocusGroupKind::Unknown => PriorityBand::Interactive,
            },
            reasons: vec![format!("resolver test group {kind:?}")],
        }
    }

    fn resolver_test_snapshot(
        groups: Vec<FocusGroup>,
        processes: Vec<FocusProcess>,
        elapsed_ms: u64,
    ) -> FocusSnapshot {
        let mut process_map = BTreeMap::new();
        let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for process in processes {
            children_by_parent
                .entry(process.ppid)
                .or_default()
                .push(process.pid);
            process_map.insert(process.pid, process);
        }

        for children in children_by_parent.values_mut() {
            children.sort_unstable();
        }

        FocusSnapshot {
            elapsed_ms,
            foreground: None,
            processes: process_map,
            children_by_parent,
            groups,
        }
    }

    fn resolver_test_policy() -> FocusPolicy {
        FocusPolicy {
            poll_ms: 1000,
            min_confidence: 0.60,
            switch_margin: 0.20,
            switch_cooldown_ms: 5000,
            required_winner_polls: 2,
            max_roots: 4,
        }
    }

    #[test]
    fn focus_resolver_requires_repeated_winner_before_initial_switch() {
        let mut resolver = FocusResolver::new(resolver_test_policy());
        let group = resolver_test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10],
            Some(10),
            0.80,
            0.80,
        );
        let process = resolver_test_process(10, "Game.exe", SystemTaskClass::Game);

        let first = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![group.clone()],
            vec![process.clone()],
            1000,
        ));

        match first {
            FocusDecision::NoTarget { reason } => {
                assert!(reason.contains("waiting for stable winner"));
            }
            other => panic!("expected NoTarget on first pending poll, got {other:?}"),
        }

        let second =
            resolver.decide_from_snapshot(resolver_test_snapshot(vec![group], vec![process], 2000));

        match second {
            FocusDecision::Switch { old, new } => {
                assert!(old.is_none());
                assert_eq!(new.group.kind, FocusGroupKind::Game);
                assert_eq!(new.group.root_pids, vec![10]);
                assert_eq!(new.selected_at_ms, 2000);
                assert_eq!(new.last_confirmed_ms, 2000);
                assert_eq!(
                    new.situation,
                    crate::autotune::state::SituationKind::GameFocused
                );
            }
            other => panic!("expected initial Switch after repeated winner, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_keeps_current_when_alive_and_score_floor_met() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        let mut resolver = FocusResolver::new(policy);

        let first_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20],
            Some(20),
            0.90,
            0.90,
        );
        let first_process =
            resolver_test_process(20, "firefox", SystemTaskClass::BrowserForeground);

        let first = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![first_group],
            vec![first_process],
            0,
        ));

        match first {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let keep_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20],
            Some(20),
            0.45,
            0.70,
        );
        let keep_process = resolver_test_process(20, "firefox", SystemTaskClass::BrowserForeground);

        let keep = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![keep_group],
            vec![keep_process],
            1000,
        ));

        match keep {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
                assert_eq!(focus.group.score, 0.45);
                assert_eq!(focus.selected_at_ms, 0);
                assert_eq!(focus.last_confirmed_ms, 1000);
            }
            other => {
                panic!("expected Keep for live current focus above score floor, got {other:?}")
            }
        }
    }

    #[test]
    fn focus_resolver_enforces_switch_margin_and_cooldown() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 5000;
        let mut resolver = FocusResolver::new(policy);

        let old_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![30],
            vec![30],
            Some(30),
            0.60,
            0.90,
        );
        let old_process = resolver_test_process(30, "firefox", SystemTaskClass::BrowserForeground);

        let initial = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone()],
            vec![old_process.clone()],
            0,
        ));

        match initial {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let weak_new_group = resolver_test_group(
            FocusGroupKind::Compile,
            vec![40],
            vec![40],
            Some(40),
            0.75,
            0.95,
        );
        let weak_new_process = resolver_test_process(40, "cargo", SystemTaskClass::BuildJob);

        let below_margin = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone(), weak_new_group],
            vec![old_process.clone(), weak_new_process],
            1000,
        ));

        match below_margin {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected Keep when candidate fails switch margin, got {other:?}"),
        }

        let strong_new_group = resolver_test_group(
            FocusGroupKind::Compile,
            vec![41],
            vec![41],
            Some(41),
            0.90,
            0.95,
        );
        let strong_new_process = resolver_test_process(41, "cargo", SystemTaskClass::BuildJob);

        let during_cooldown = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone(), strong_new_group.clone()],
            vec![old_process.clone(), strong_new_process.clone()],
            2000,
        ));

        match during_cooldown {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected Keep during switch cooldown, got {other:?}"),
        }

        let after_cooldown = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group, strong_new_group],
            vec![old_process, strong_new_process],
            6000,
        ));

        match after_cooldown {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
                assert_eq!(
                    new.situation,
                    crate::autotune::state::SituationKind::CompileLoad
                );
            }
            other => panic!("expected Switch after cooldown and margin, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_clears_when_no_group_meets_confidence() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        let mut resolver = FocusResolver::new(policy);

        let high_conf_group = resolver_test_group(
            FocusGroupKind::Game,
            vec![50],
            vec![50],
            Some(50),
            0.90,
            0.90,
        );
        let process = resolver_test_process(50, "Game.exe", SystemTaskClass::Game);

        let initial = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![high_conf_group],
            vec![process.clone()],
            0,
        ));

        match initial {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Game);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let low_conf_group = resolver_test_group(
            FocusGroupKind::Game,
            vec![50],
            vec![50],
            Some(50),
            0.90,
            0.40,
        );

        let clear = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![low_conf_group],
            vec![process],
            1000,
        ));

        match clear {
            FocusDecision::Clear { old, reason } => {
                assert!(old.is_some());
                assert!(reason.contains("min_confidence"));
            }
            other => panic!("expected Clear when no group meets confidence, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_limits_selected_root_pids_to_policy_max_roots() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        policy.max_roots = 2;
        let mut resolver = FocusResolver::new(policy);

        let group = resolver_test_group(
            FocusGroupKind::Game,
            vec![60, 61, 62],
            vec![60, 61, 62],
            Some(60),
            0.90,
            0.90,
        );

        let processes = vec![
            resolver_test_process(60, "GameA.exe", SystemTaskClass::Game),
            resolver_test_process(61, "GameB.exe", SystemTaskClass::Game),
            resolver_test_process(62, "GameC.exe", SystemTaskClass::Game),
        ];

        let decision =
            resolver.decide_from_snapshot(resolver_test_snapshot(vec![group], processes, 0));

        match decision {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.root_pids, vec![60, 61]);
                assert_eq!(new.group.member_pids, vec![60, 61, 62]);
            }
            other => panic!("expected Switch with truncated roots, got {other:?}"),
        }
    }

    fn test_classification(
        class: SystemTaskClass,
        priority_band: PriorityBand,
        confidence: f32,
    ) -> Classification {
        Classification {
            class,
            priority_band,
            confidence,
            reasons: vec![format!("test classification {class:?}")],
        }
    }

    fn test_process(
        pid: u32,
        ppid: u32,
        comm: &str,
        class: SystemTaskClass,
        priority_band: PriorityBand,
        cpu_time_ticks_delta: u64,
    ) -> FocusProcess {
        FocusProcess {
            pid,
            ppid,
            comm: comm.to_owned(),
            cmdline: comm.to_owned(),
            cgroup_path: None,
            starttime_ticks: Some(pid as u64 * 10),
            sched_policy: None,
            is_foreground_window_process: false,
            classification: test_classification(class, priority_band, 0.9),
            cpu_time_ticks_delta,
            read_bytes_delta: 0,
            write_bytes_delta: 0,
            voluntary_ctxt_switches_delta: 0,
            nonvoluntary_ctxt_switches_delta: 0,
        }
    }

    fn test_snapshot(processes: Vec<FocusProcess>) -> FocusSnapshot {
        let mut process_map = BTreeMap::new();
        let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for process in processes {
            children_by_parent
                .entry(process.ppid)
                .or_default()
                .push(process.pid);
            process_map.insert(process.pid, process);
        }

        for children in children_by_parent.values_mut() {
            children.sort_unstable();
        }

        FocusSnapshot {
            elapsed_ms: 1000,
            foreground: None,
            processes: process_map,
            children_by_parent,
            groups: Vec::new(),
        }
    }

    #[test]
    fn focus_groups_prefer_stable_compile_root_over_compiler_children() {
        let snapshot = test_snapshot(vec![
            test_process(
                10,
                1,
                "foot",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                1,
            ),
            test_process(
                11,
                10,
                "cargo",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                2,
            ),
            test_process(
                12,
                11,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                80,
            ),
            test_process(
                13,
                11,
                "ld.lld",
                SystemTaskClass::Linker,
                PriorityBand::Throughput,
                25,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![11]);
        assert_eq!(compile.primary_pid, Some(11));
        assert_eq!(compile.member_pids, vec![11, 12, 13]);
    }

    #[test]
    fn focus_groups_group_orphan_compilers_under_nearest_terminal_session() {
        let snapshot = test_snapshot(vec![
            test_process(
                20,
                1,
                "kitty",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                3,
            ),
            test_process(
                21,
                20,
                "zsh",
                SystemTaskClass::Shell,
                PriorityBand::Interactive,
                4,
            ),
            test_process(
                22,
                21,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                60,
            ),
            test_process(
                23,
                21,
                "clang",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                50,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![21]);
        assert_eq!(compile.primary_pid, Some(22));
        assert_eq!(compile.member_pids, vec![21, 22, 23]);
    }

    #[test]
    fn focus_groups_root_browser_at_parent_not_idle_renderer() {
        let snapshot = test_snapshot(vec![
            test_process(
                30,
                1,
                "firefox",
                SystemTaskClass::BrowserForeground,
                PriorityBand::ForegroundLatency,
                5,
            ),
            test_process(
                31,
                30,
                "Web Content",
                SystemTaskClass::BrowserRenderer,
                PriorityBand::Interactive,
                100,
            ),
            test_process(
                32,
                30,
                "GPU Process",
                SystemTaskClass::BrowserGpu,
                PriorityBand::Interactive,
                20,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert_eq!(browser.root_pids, vec![30]);
        assert_eq!(browser.primary_pid, Some(30));
        assert_eq!(browser.member_pids, vec![30, 31, 32]);
    }

    #[test]
    fn focus_groups_include_wineserver_tied_to_game_runtime() {
        let mut game = test_process(
            40,
            1,
            "pressure-vessel",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            10,
        );
        game.cmdline = "/home/user/.steam/steamapps/common/Game/pressure-vessel".to_owned();
        game.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut game_child = test_process(
            41,
            40,
            "Game.exe",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            120,
        );
        game_child.cmdline = "/home/user/.steam/steamapps/common/Game/Game.exe".to_owned();
        game_child.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut wineserver = test_process(
            42,
            1,
            "wineserver",
            SystemTaskClass::WineServer,
            PriorityBand::ForegroundLatency,
            15,
        );
        wineserver.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let snapshot = test_snapshot(vec![game, game_child, wineserver]);

        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert_eq!(game_group.root_pids, vec![40]);
        assert_eq!(game_group.primary_pid, Some(40));
        assert_eq!(game_group.member_pids, vec![40, 41, 42]);
    }

    #[test]
    fn focus_group_score_is_clamped_and_exposes_breakdown() {
        let cargo = test_process(
            600,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            10_000,
        );
        let rustc = test_process(
            601,
            600,
            "rustc",
            SystemTaskClass::Compiler,
            PriorityBand::Throughput,
            10_000,
        );

        let snapshot = test_snapshot(vec![cargo, rustc]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.score, 1.0);
        assert!(compile.score_breakdown.cpu_score > 0.0);
        assert!(compile.score_breakdown.class_priority_score > 0.0);
        assert!(compile.score_breakdown.stability_score > 0.0);
    }

    #[test]
    fn focus_group_confidence_is_not_high_from_name_only() {
        let cargo = test_process(
            610,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            0,
        );

        let snapshot = test_snapshot(vec![cargo]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert!(compile.score_breakdown.class_priority_score > 0.0);
        assert_eq!(compile.score_breakdown.cpu_score, 0.0);
        assert!(compile.confidence <= 0.55);
    }

    #[test]
    fn focus_group_penalizes_indexer_only_compile_group() {
        let clangd = test_process(
            620,
            1,
            "clangd",
            SystemTaskClass::Indexer,
            PriorityBand::Background,
            25,
        );

        let snapshot = test_snapshot(vec![clangd]);
        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert!(compile.score_breakdown.penalty >= 0.55);
        assert!(compile.score < 0.50);
    }

    #[test]
    fn focus_group_scores_game_from_runtime_and_active_descendants() {
        let mut runtime = test_process(
            630,
            1,
            "pressure-vessel",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            5,
        );
        runtime.cmdline = "/home/user/.steam/steamapps/common/Game/pressure-vessel".to_owned();

        let mut game = test_process(
            631,
            630,
            "Game.exe",
            SystemTaskClass::GameRenderThread,
            PriorityBand::ForegroundLatency,
            80,
        );
        game.cmdline = "/home/user/.steam/steamapps/common/Game/Game.exe".to_owned();

        let snapshot = test_snapshot(vec![runtime, game]);
        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert!(game_group.score > 0.50);
        assert!(game_group.confidence > 0.55);
        assert!(game_group.score_breakdown.class_priority_score > 0.0);
        assert_eq!(game_group.score_breakdown.penalty, 0.0);
    }

    #[test]
    fn focus_group_penalizes_launcher_only_game_group() {
        let steam = test_process(
            640,
            1,
            "steam",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            0,
        );

        let snapshot = test_snapshot(vec![steam]);
        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert!(game_group.score_breakdown.penalty >= 0.20);
        assert!(game_group.confidence <= 0.55);
    }

    #[test]
    fn focus_group_scores_browser_from_active_children() {
        let parent = test_process(
            650,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            5,
        );
        let renderer = test_process(
            651,
            650,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            60,
        );
        let gpu = test_process(
            652,
            650,
            "GPU Process",
            SystemTaskClass::BrowserGpu,
            PriorityBand::Interactive,
            20,
        );

        let snapshot = test_snapshot(vec![parent, renderer, gpu]);
        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert!(browser.score > 0.40);
        assert!(browser.confidence > 0.55);
        assert_eq!(browser.score_breakdown.penalty, 0.0);
    }

    #[test]
    fn focus_group_penalizes_many_idle_browser_renderers() {
        let parent = test_process(
            660,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            1,
        );
        let renderer_one = test_process(
            661,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_two = test_process(
            662,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_three = test_process(
            663,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );
        let renderer_four = test_process(
            664,
            660,
            "Web Content",
            SystemTaskClass::BrowserRenderer,
            PriorityBand::Interactive,
            0,
        );

        let snapshot = test_snapshot(vec![
            parent,
            renderer_one,
            renderer_two,
            renderer_three,
            renderer_four,
        ]);
        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert!(browser.score_breakdown.penalty > 0.0);
        assert!(browser.confidence <= 0.75);
    }

    #[test]
    fn focus_groups_do_not_let_idle_steam_beat_active_compile() {
        let mut steam = test_process(
            50,
            1,
            "steam",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        steam.cmdline = "steam".to_owned();

        let cargo = test_process(
            60,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            30,
        );
        let rustc = test_process(
            61,
            60,
            "rustc",
            SystemTaskClass::Compiler,
            PriorityBand::Throughput,
            90,
        );

        let snapshot = test_snapshot(vec![steam, cargo, rustc]);

        let groups = build_focus_groups(&snapshot);

        assert_eq!(groups.first().unwrap().kind, FocusGroupKind::Compile);
        assert!(
            groups
                .iter()
                .position(|group| group.kind == FocusGroupKind::Idle)
                .unwrap()
                > groups
                    .iter()
                    .position(|group| group.kind == FocusGroupKind::Compile)
                    .unwrap()
        );
    }

    #[test]
    fn focus_groups_fallback_selects_highest_non_service_interactive_tree_by_cpu() {
        let service = test_process(
            70,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            500,
        );
        let editor = test_process(
            80,
            1,
            "nvim",
            SystemTaskClass::Editor,
            PriorityBand::Interactive,
            20,
        );
        let terminal = test_process(
            90,
            1,
            "foot",
            SystemTaskClass::Terminal,
            PriorityBand::Interactive,
            60,
        );

        let mut snapshot = test_snapshot(vec![service, editor, terminal]);
        for process in snapshot.processes.values_mut() {
            process.classification.class = SystemTaskClass::Unknown;
        }
        snapshot
            .processes
            .get_mut(&70)
            .unwrap()
            .classification
            .class = SystemTaskClass::Service;
        snapshot
            .processes
            .get_mut(&80)
            .unwrap()
            .classification
            .class = SystemTaskClass::Editor;
        snapshot
            .processes
            .get_mut(&90)
            .unwrap()
            .classification
            .class = SystemTaskClass::Terminal;

        let groups = build_focus_groups(&snapshot);
        let fallback = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Unknown)
            .unwrap();

        assert_eq!(fallback.root_pids, vec![90]);
        assert_eq!(fallback.primary_pid, Some(90));
        assert_eq!(fallback.member_pids, vec![90]);
    }

    #[test]
    fn gaming_wins_over_idle_steam() {
        let cgroup = "/user.slice/app-steam-game.scope";
        let first_sample = vec![
            FakeProcProcess::new(100, 1, "steam", "steam").with_cgroup("/user.slice/steam.scope"),
            FakeProcProcess::new(
                110,
                99, // Use a different PPID so they aren't siblings
                "pressure-vessel",
                "/home/user/.steam/steamapps/common/Game/pressure-vessel",
            )
            .with_cgroup(cgroup),
            FakeProcProcess::new(
                111,
                110,
                "Game.exe",
                "/home/user/.steam/steamapps/common/Game/Game.exe",
            )
            .with_cgroup(cgroup),
        ];
        let second_sample = vec![
            FakeProcProcess::new(100, 1, "steam", "steam").with_cgroup("/user.slice/steam.scope"),
            FakeProcProcess::new(
                110,
                99,
                "pressure-vessel",
                "/home/user/.steam/steamapps/common/Game/pressure-vessel",
            )
            .with_cgroup(cgroup)
            .with_activity(5, 0, 0, 1, 0),
            FakeProcProcess::new(
                111,
                110,
                "Game.exe",
                "/home/user/.steam/steamapps/common/Game/Game.exe",
            )
            .with_cgroup(cgroup)
            .with_activity(180, 0, 0, 20, 5),
        ];

        let snapshot = focus_snapshot_from_fake_proc(
            "gaming_wins_over_idle_steam",
            first_sample,
            second_sample,
        );

        assert_eq!(snapshot.groups.first().unwrap().kind, FocusGroupKind::Game);
        let game_group = first_focus_group(&snapshot, FocusGroupKind::Game);
        assert_eq!(game_group.root_pids, vec![110]);
        assert_eq!(game_group.primary_pid, Some(110));
        assert!(game_group.member_pids.contains(&110));
        assert!(game_group.member_pids.contains(&111));
        assert!(!game_group.member_pids.contains(&100));
    }

    #[test]
    fn idle_launcher_does_not_win() {
        let first_sample = vec![
            FakeProcProcess::new(200, 1, "steam", "steam"),
            FakeProcProcess::new(210, 1, "firefox", "firefox"),
            FakeProcProcess::new(211, 210, "firefox: Web Content", "firefox web content"),
            FakeProcProcess::new(
                212,
                210,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            ),
        ];
        let second_sample = vec![
            FakeProcProcess::new(200, 1, "steam", "steam"),
            FakeProcProcess::new(210, 1, "firefox", "firefox").with_activity(15, 0, 0, 8, 1),
            FakeProcProcess::new(211, 210, "firefox: Web Content", "firefox web content")
                .with_activity(160, 0, 0, 30, 2),
            FakeProcProcess::new(
                212,
                210,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            )
            .with_activity(30, 0, 0, 12, 1),
        ];

        let snapshot = focus_snapshot_from_fake_proc(
            "idle_launcher_does_not_win",
            first_sample,
            second_sample,
        );

        assert_eq!(
            snapshot.groups.first().unwrap().kind,
            FocusGroupKind::Browser
        );
        let browser_group = first_focus_group(&snapshot, FocusGroupKind::Browser);
        assert_eq!(browser_group.root_pids, vec![210]);
        assert_eq!(browser_group.primary_pid, Some(210));
        assert!(browser_group.member_pids.contains(&211));
        assert!(browser_group.member_pids.contains(&212));
    }

    #[test]
    fn compile_root_is_stable() {
        let first_sample = vec![
            FakeProcProcess::new(300, 1, "cargo", "cargo build"),
            FakeProcProcess::new(301, 300, "rustc", "rustc crate_a"),
            FakeProcProcess::new(302, 300, "rustc", "rustc crate_b"),
        ];
        let second_sample = vec![
            FakeProcProcess::new(300, 1, "cargo", "cargo build").with_activity(20, 0, 0, 8, 1),
            FakeProcProcess::new(301, 300, "rustc", "rustc crate_a")
                .with_activity(200, 0, 0, 10, 2),
            FakeProcProcess::new(302, 300, "rustc", "rustc crate_b").with_activity(180, 0, 0, 9, 2),
        ];

        let snapshot =
            focus_snapshot_from_fake_proc("compile_root_is_stable", first_sample, second_sample);

        let compile_group = first_focus_group(&snapshot, FocusGroupKind::Compile);
        assert_eq!(compile_group.root_pids, vec![300]);
        assert_eq!(compile_group.primary_pid, Some(300));
        assert_eq!(compile_group.member_pids, vec![300, 301, 302]);
        assert!(!compile_group.root_pids.contains(&301));
        assert!(!compile_group.root_pids.contains(&302));
    }

    #[test]
    fn linker_pressure_sets_compile_linker_situation() {
        let first_sample = vec![
            FakeProcProcess::new(400, 1, "cargo", "cargo build"),
            FakeProcProcess::new(401, 400, "rustc", "rustc crate_a"),
            FakeProcProcess::new(402, 400, "ld.lld", "ld.lld -o target/debug/app"),
        ];
        let second_sample =
            vec![
                FakeProcProcess::new(400, 1, "cargo", "cargo build").with_activity(20, 0, 0, 8, 1),
                FakeProcProcess::new(401, 400, "rustc", "rustc crate_a")
                    .with_activity(120, 0, 0, 10, 2),
                FakeProcProcess::new(402, 400, "ld.lld", "ld.lld -o target/debug/app")
                    .with_activity(250, 64 * 1024 * 1024, 128 * 1024 * 1024, 8, 2),
            ];

        let snapshot = focus_snapshot_from_fake_proc(
            "linker_pressure_sets_compile_linker_situation",
            first_sample,
            second_sample,
        );

        let compile_group = first_focus_group(&snapshot, FocusGroupKind::Compile);
        assert_eq!(compile_group.root_pids, vec![400]);
        assert!(compile_group.member_pids.contains(&402));
        assert!(compile_group.score_breakdown.io_score > 0.0);
        assert!(
            compile_group
                .reasons
                .iter()
                .any(|reason| reason.contains("compile group prefers stable build roots"))
        );
        assert!(matches!(
            situation_for_group(compile_group),
            SituationKind::CompileLoad | SituationKind::CompileLinkerPressure
        ));
    }

    #[test]
    fn browser_renderer_grouping() {
        let first_sample = vec![
            FakeProcProcess::new(500, 1, "firefox", "firefox"),
            FakeProcProcess::new(
                501,
                500,
                "firefox: Web Content",
                "firefox isolated web content",
            ),
            FakeProcProcess::new(502, 500, "firefox: Web Content", "firefox web content tab"),
            FakeProcProcess::new(
                503,
                500,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            ),
        ];
        let second_sample = vec![
            FakeProcProcess::new(500, 1, "firefox", "firefox").with_activity(10, 0, 0, 5, 1),
            FakeProcProcess::new(
                501,
                500,
                "firefox: Web Content",
                "firefox isolated web content",
            )
            .with_activity(90, 0, 0, 20, 2),
            FakeProcProcess::new(502, 500, "firefox: Web Content", "firefox web content tab")
                .with_activity(70, 0, 0, 16, 2),
            FakeProcProcess::new(
                503,
                500,
                "firefox: GPU Process",
                "firefox --type=gpu-process",
            )
            .with_activity(30, 0, 0, 8, 1),
        ];

        let snapshot =
            focus_snapshot_from_fake_proc("browser_renderer_grouping", first_sample, second_sample);

        let browser_group = first_focus_group(&snapshot, FocusGroupKind::Browser);
        assert_eq!(browser_group.root_pids, vec![500]);
        assert_eq!(browser_group.primary_pid, Some(500));
        assert_eq!(browser_group.member_pids, vec![500, 501, 502, 503]);
    }

    #[test]
    fn hysteresis_keeps_current_focus() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![600],
            vec![600],
            Some(600),
            0.70,
            0.90,
        );
        let browser_process = required_test_process(
            600,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            30,
        );

        let initial = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            0,
        ));

        assert!(matches!(initial, FocusDecision::Switch { .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![700],
            vec![700],
            Some(700),
            0.75,
            0.95,
        );
        let compile_process = required_test_process(
            700,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            200,
        );

        let decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group, compile_group],
            vec![browser_process, compile_process],
            1000,
        ));

        match decision {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
                assert_eq!(focus.group.score, 0.70);
            }
            other => panic!("expected browser focus to be kept, got {other:?}"),
        }
    }

    #[test]
    fn sustained_winner_switches() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 2;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![800],
            vec![800],
            Some(800),
            0.60,
            0.90,
        );
        let browser_process = required_test_process(
            800,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            30,
        );

        let first = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            0,
        ));
        assert!(matches!(first, FocusDecision::NoTarget { .. }));

        let second = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            1000,
        ));
        assert!(matches!(second, FocusDecision::Switch { old: None, .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![900],
            vec![900],
            Some(900),
            0.95,
            0.95,
        );
        let compile_process = required_test_process(
            900,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            300,
        );

        let pending = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group, compile_group.clone()],
            vec![browser_process, compile_process.clone()],
            2000,
        ));

        match pending {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected keep while compile winner is pending, got {other:?}"),
        }

        let switched = resolver.decide_from_snapshot(required_test_snapshot(
            vec![compile_group],
            vec![compile_process],
            3000,
        ));

        match switched {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
                assert_eq!(new.group.score, 0.95);
            }
            other => panic!("expected sustained compile winner to switch, got {other:?}"),
        }
    }

    #[test]
    fn idle_has_no_target() {
        let mut policy = required_focus_policy();
        policy.min_confidence = 0.80;
        let mut resolver = FocusResolver::new(policy);

        let idle_group = required_test_group(
            FocusGroupKind::Idle,
            vec![1000],
            vec![1000],
            Some(1000),
            0.10,
            0.30,
        );
        let idle_process = required_test_process(
            1000,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );

        let decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![idle_group],
            vec![idle_process],
            1000,
        ));

        match decision {
            FocusDecision::NoTarget { reason } => {
                assert!(reason.contains("min_confidence"));
            }
            other => panic!("expected NoTarget for idle below threshold, got {other:?}"),
        }
    }

    #[test]
    fn max_roots_is_enforced() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.max_roots = 2;
        let mut resolver = FocusResolver::new(policy);

        let group = required_test_group(
            FocusGroupKind::Compile,
            vec![1100, 1101, 1102, 1103, 1104],
            vec![1100, 1101, 1102, 1103, 1104],
            Some(1100),
            0.95,
            0.95,
        );
        let processes = vec![
            required_test_process(
                1100,
                1,
                "cargo-a",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1101,
                1,
                "cargo-b",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1102,
                1,
                "cargo-c",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1103,
                1,
                "cargo-d",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1104,
                1,
                "cargo-e",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
        ];

        let decision =
            resolver.decide_from_snapshot(required_test_snapshot(vec![group], processes, 1000));

        match decision {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.root_pids, vec![1100, 1101]);
                assert_eq!(new.group.member_pids, vec![1100, 1101, 1102, 1103, 1104]);
            }
            other => panic!("expected Switch with capped roots, got {other:?}"),
        }
    }

    #[test]
    fn dead_root_clears_or_switches() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![1200],
            vec![1200],
            Some(1200),
            0.80,
            0.90,
        );
        let browser_process = required_test_process(
            1200,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            100,
        );

        let initial = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group],
            vec![browser_process],
            0,
        ));
        assert!(matches!(initial, FocusDecision::Switch { .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![1300],
            vec![1300],
            Some(1300),
            0.95,
            0.95,
        );
        let compile_process = required_test_process(
            1300,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            200,
        );

        let switch_decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![compile_group],
            vec![compile_process],
            1000,
        ));

        match switch_decision {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
            }
            FocusDecision::Clear { old, reason } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert!(reason.contains("current focus root disappeared"));
            }
            other => panic!("expected dead root to switch or clear cleanly, got {other:?}"),
        }
    }

    #[test]
    fn audio_realtime_priority_band() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1400,
            ppid: 1,
            comm: "pipewire",
            cmdline: "pipewire",
            exe_path: None,
            cgroup_path: None,
            sched_policy: Some(SCHED_FIFO),
        });

        assert_eq!(classification.class, SystemTaskClass::AudioRealtime);
        assert_eq!(classification.priority_band, PriorityBand::CriticalRealtime);
        assert!(classification.confidence >= 0.60);

        assert_eq!(
            priority_band_for_class(SystemTaskClass::AudioRealtime, Some(SCHED_RR)),
            PriorityBand::CriticalRealtime
        );
    }

    #[test]
    fn compositor_not_background() {
        for comm in ["sway", "kwin_wayland", "mutter", "gnome-shell"] {
            let classification = classify_process(&ProcessIdentity {
                pid: 1500,
                ppid: 1,
                comm,
                cmdline: comm,
                exe_path: None,
                cgroup_path: None,
                sched_policy: None,
            });

            assert_eq!(classification.class, SystemTaskClass::Compositor);
            assert_eq!(
                classification.priority_band,
                PriorityBand::ForegroundLatency
            );
            assert_ne!(classification.priority_band, PriorityBand::Background);
        }

        assert_eq!(
            priority_band_for_class(SystemTaskClass::Compositor, None),
            PriorityBand::ForegroundLatency
        );
    }

    #[test]
    fn focus_classification_uses_community_rule_reason_for_proton_game() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1600,
            ppid: 1,
            comm: "KingdomCome",
            cmdline: "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe --game",
            exe_path: Some("/usr/bin/wine"),
            cgroup_path: Some("/user.slice/app-steam-379430.scope"),
            sched_policy: None,
        });

        assert_eq!(classification.class, SystemTaskClass::Game);
        assert_eq!(
            classification.priority_band,
            PriorityBand::ForegroundLatency
        );
        assert!(
            classification.reasons.iter().any(|reason| {
                reason.contains("community-rules") && reason.contains("wine_proton_k.rules")
            }),
            "reasons={:?}",
            classification.reasons
        );
    }

    #[test]
    fn focus_ambiguous_exe_without_context_is_not_game() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1601,
            ppid: 1,
            comm: "build.exe",
            cmdline: "/tmp/build.exe --compile",
            exe_path: Some("/tmp/build.exe"),
            cgroup_path: Some("/user.slice/app-builder.scope"),
            sched_policy: None,
        });

        assert_ne!(classification.class, SystemTaskClass::Game);
    }

    #[test]
    fn focus_hardcoded_audio_classification_wins_over_community_context() {
        let classification = classify_process(&ProcessIdentity {
            pid: 1602,
            ppid: 1,
            comm: "pipewire",
            cmdline: "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
            exe_path: Some("/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe"),
            cgroup_path: Some("/user.slice/app-steam-379430.scope"),
            sched_policy: Some(SCHED_FIFO),
        });

        assert_eq!(classification.class, SystemTaskClass::AudioRealtime);
        assert!(
            classification
                .reasons
                .iter()
                .all(|reason| !reason.contains("community-rules")),
            "reasons={:?}",
            classification.reasons
        );
    }

    #[test]
    fn focus_counter_deltas_are_zero_on_first_seen_and_reset_on_pid_reuse() {
        let current = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 100,
            read_bytes: 200,
            write_bytes: 300,
            voluntary_ctxt_switches: 40,
            nonvoluntary_ctxt_switches: 50,
        };

        let first_seen = counter_deltas(None, &current);
        assert_eq!(first_seen.starttime_ticks, Some(10));
        assert_eq!(first_seen.cpu_time_ticks, 0);
        assert_eq!(first_seen.read_bytes, 0);
        assert_eq!(first_seen.write_bytes, 0);
        assert_eq!(first_seen.voluntary_ctxt_switches, 0);
        assert_eq!(first_seen.nonvoluntary_ctxt_switches, 0);

        let previous = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 70,
            read_bytes: 125,
            write_bytes: 250,
            voluntary_ctxt_switches: 35,
            nonvoluntary_ctxt_switches: 45,
        };

        let deltas = counter_deltas(Some(&previous), &current);
        assert_eq!(deltas.starttime_ticks, Some(10));
        assert_eq!(deltas.cpu_time_ticks, 30);
        assert_eq!(deltas.read_bytes, 75);
        assert_eq!(deltas.write_bytes, 50);
        assert_eq!(deltas.voluntary_ctxt_switches, 5);
        assert_eq!(deltas.nonvoluntary_ctxt_switches, 5);

        let reused_pid_previous = FocusCounters {
            starttime_ticks: Some(9),
            cpu_time_ticks: 500,
            read_bytes: 600,
            write_bytes: 700,
            voluntary_ctxt_switches: 80,
            nonvoluntary_ctxt_switches: 90,
        };

        let reused_pid_deltas = counter_deltas(Some(&reused_pid_previous), &current);
        assert_eq!(reused_pid_deltas.starttime_ticks, Some(10));
        assert_eq!(reused_pid_deltas.cpu_time_ticks, 0);
        assert_eq!(reused_pid_deltas.read_bytes, 0);
        assert_eq!(reused_pid_deltas.write_bytes, 0);
        assert_eq!(reused_pid_deltas.voluntary_ctxt_switches, 0);
        assert_eq!(reused_pid_deltas.nonvoluntary_ctxt_switches, 0);
    }

    #[test]
    fn focus_group_kind_maps_system_classes() {
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::GameRenderThread),
            FocusGroupKind::Game
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::BrowserGpu),
            FocusGroupKind::Browser
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compiler),
            FocusGroupKind::Compile
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Media),
            FocusGroupKind::Media
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Recorder),
            FocusGroupKind::Recording
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::VirtualMachine),
            FocusGroupKind::VirtualMachine
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compositor),
            FocusGroupKind::Desktop
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Service),
            FocusGroupKind::Idle
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Unknown),
            FocusGroupKind::Unknown
        );
    }

    #[test]
    fn legacy_task_class_maps_game_related_system_classes_to_game() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Game),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameRenderThread),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameWorkerThread),
            TaskClass::Game
        );
    }

    #[test]
    fn legacy_task_class_preserves_special_foreground_classes() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::WineServer),
            TaskClass::WineServer
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameScope),
            TaskClass::GameScope
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Compositor),
            TaskClass::Compositor
        );
    }

    #[test]
    fn legacy_task_class_maps_daemon_and_kernel_classes_to_service() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Service),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::StorageDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::NetworkDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::KernelThread),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::IrqThread),
            TaskClass::Service
        );
    }

    #[test]
    fn legacy_task_class_maps_all_other_system_classes_to_helper() {
        let classes = [
            SystemTaskClass::AudioRealtime,
            SystemTaskClass::Input,
            SystemTaskClass::BrowserForeground,
            SystemTaskClass::BrowserBackground,
            SystemTaskClass::BrowserRenderer,
            SystemTaskClass::BrowserGpu,
            SystemTaskClass::BrowserNetwork,
            SystemTaskClass::BuildJob,
            SystemTaskClass::Compiler,
            SystemTaskClass::Linker,
            SystemTaskClass::Indexer,
            SystemTaskClass::PackageManager,
            SystemTaskClass::Editor,
            SystemTaskClass::Terminal,
            SystemTaskClass::Shell,
            SystemTaskClass::Media,
            SystemTaskClass::Recorder,
            SystemTaskClass::VirtualMachine,
            SystemTaskClass::Unknown,
        ];

        for class in classes {
            assert_eq!(legacy_task_class_for_system_class(class), TaskClass::Helper);
        }
    }
}
