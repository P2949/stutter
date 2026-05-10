use super::*;

#[derive(Debug, Clone)]
pub struct FocusPolicy {
    pub poll_ms: u64,
    pub min_confidence: f32,
    pub switch_margin: f32,
    pub switch_cooldown_ms: u64,
    pub required_winner_polls: u32,
    pub max_roots: usize,
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
                    "waiting for stable winner poll {}/{} first_seen_ms={}",
                    self.pending
                        .as_ref()
                        .map(|pending| pending.polls)
                        .unwrap_or(0),
                    self.policy.required_winner_polls.max(1),
                    self.pending
                        .as_ref()
                        .map(|pending| pending.first_seen_ms)
                        .unwrap_or(snapshot.elapsed_ms)
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
