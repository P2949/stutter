//! Strict qualification for the bounded autonomous service. Observation-only focus retains its
//! existing hysteresis; a mutator clears its target when that hysteresis would hide uncertainty.
use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::{
    focus::{FocusGroupKind, tree_walk::has_ancestor_in_set},
    process_tree::TaskClass,
};

impl FocusResolver {
    pub(crate) fn qualified_root_starttimes(&self) -> BTreeMap<u32, u64> {
        self.qualified_root_identities.clone()
    }
    pub(crate) fn bounded_autotune(mut self, min_confidence: f32) -> Self {
        self.policy.min_confidence = self.policy.min_confidence.max(min_confidence);
        self.autonomous_qualification = true;
        self
    }

    fn unqualify(&mut self, reason: &str) -> FocusDecision {
        self.pending = None;
        self.qualified_root_identities.clear();
        match self.current.take() {
            Some(old) => FocusDecision::Clear {
                old: Some(old),
                reason: reason.to_owned(),
            },
            None => FocusDecision::NoTarget {
                reason: reason.to_owned(),
            },
        }
    }

    pub(super) fn qualify_autonomous_snapshot(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Option<FocusDecision> {
        // Inspect complete groups before max_roots truncation. Multiple independent games must
        // not become an apparently unique target merely because the output limit is one root.
        let games: BTreeSet<u32> = snapshot
            .groups
            .iter()
            .filter(|group| {
                group.kind == FocusGroupKind::Game && group.confidence >= self.policy.min_confidence
            })
            .flat_map(|group| group.root_pids.iter().copied())
            .filter(|pid| snapshot.processes.contains_key(pid))
            .collect();
        // A Game group names one root and folds every other game-like process of the same
        // parent into its members, so two independent games started by one launcher are still
        // one group. Count the independent top-level game processes inside the groups too.
        let game_class: BTreeSet<u32> = snapshot
            .groups
            .iter()
            .filter(|group| {
                group.kind == FocusGroupKind::Game && group.confidence >= self.policy.min_confidence
            })
            .flat_map(|group| group.member_pids.iter().copied())
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .is_some_and(|process| process.classification.class == TaskClass::Game)
            })
            .collect();
        let independent = game_class
            .iter()
            .filter(|pid| !has_ancestor_in_set(snapshot, **pid, &game_class))
            .count();
        if games.len() > 1 || independent > 1 {
            return Some(self.unqualify(
                "ambiguous_game_roots: autonomous focus requires one independent game",
            ));
        }
        let Some(candidate) = self.best_eligible_group(snapshot) else {
            return Some(self.unqualify("focus_unqualified: no high-confidence live winner"));
        };
        let roots = if candidate.root_pids.is_empty() {
            candidate.primary_pid.into_iter().collect()
        } else {
            candidate.root_pids.clone()
        };
        let identities: BTreeMap<u32, u64> = roots
            .iter()
            .filter_map(|pid| {
                snapshot
                    .processes
                    .get(pid)?
                    .starttime_ticks
                    .filter(|ticks| *ticks > 0)
                    .map(|ticks| (*pid, ticks))
            })
            .collect();
        if roots.is_empty() || identities.len() != roots.len() {
            return Some(
                self.unqualify("focus_identity_missing: autonomous roots need PID and starttime"),
            );
        }
        let replaced = identities.iter().any(|(pid, ticks)| {
            self.qualified_root_identities
                .get(pid)
                .is_some_and(|old| old != ticks)
        });
        if replaced {
            return Some(self.unqualify(
                "focus_identity_replaced: stable polls must restart for the new process",
            ));
        }
        if let Some(current) = &self.current {
            if !Self::same_focus_identity(&current.group, &candidate) {
                return Some(
                    self.unqualify("focus_winner_changed: waiting for a newly stable winner"),
                );
            }
        } else if identities != self.qualified_root_identities {
            self.pending = None;
        }
        self.qualified_root_identities = identities;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        focus::test_support::{
            resolver_test_group, resolver_test_policy, resolver_test_process,
            resolver_test_snapshot,
        },
        process_tree::TaskClass,
    };

    fn snapshot(pid: u32, ticks: u64, elapsed: u64) -> FocusSnapshot {
        let mut process = resolver_test_process(pid, "Game.exe", TaskClass::Game);
        process.starttime_ticks = Some(ticks);
        resolver_test_snapshot(
            vec![resolver_test_group(
                FocusGroupKind::Game,
                vec![pid],
                vec![pid],
                Some(pid),
                0.95,
                0.95,
            )],
            vec![process],
            elapsed,
        )
    }
    #[test]
    fn service_requires_stability_again_after_pid_identity_replacement() {
        let mut resolver = FocusResolver::new(resolver_test_policy()).bounded_autotune(0.85);
        assert!(matches!(
            resolver.decide_from_snapshot(snapshot(10, 100, 1000)),
            FocusDecision::NoTarget { .. }
        ));
        assert!(matches!(
            resolver.decide_from_snapshot(snapshot(10, 100, 2000)),
            FocusDecision::Switch { .. }
        ));
        assert!(matches!(
            resolver.decide_from_snapshot(snapshot(10, 101, 3000)),
            FocusDecision::Clear { .. }
        ));
        assert!(matches!(
            resolver.decide_from_snapshot(snapshot(10, 101, 4000)),
            FocusDecision::NoTarget { .. }
        ));
        assert!(matches!(
            resolver.decide_from_snapshot(snapshot(10, 101, 5000)),
            FocusDecision::Switch { .. }
        ));
    }
    #[test]
    fn ambiguity_is_checked_before_root_limit_and_low_confidence_never_keeps_an_old_winner() {
        let mut policy = resolver_test_policy();
        policy.max_roots = 1;
        let mut resolver = FocusResolver::new(policy).bounded_autotune(0.85);
        resolver.decide_from_snapshot(snapshot(10, 100, 1000));
        resolver.decide_from_snapshot(snapshot(10, 100, 2000));
        let mut ambiguous = snapshot(10, 100, 3000);
        ambiguous.groups[0].root_pids.push(20);
        ambiguous.processes.insert(
            20,
            resolver_test_process(20, "OtherGame.exe", TaskClass::Game),
        );
        assert!(
            matches!(resolver.decide_from_snapshot(ambiguous), FocusDecision::Clear { reason, .. } if reason.contains("ambiguous_game_roots"))
        );
        let mut low = snapshot(10, 100, 4000);
        low.groups[0].confidence = 0.7;
        assert!(matches!(
            resolver.decide_from_snapshot(low),
            FocusDecision::NoTarget { .. }
        ));
        let mut missing = snapshot(10, 100, 5000);
        missing.processes.get_mut(&10).unwrap().starttime_ticks = None;
        assert!(
            matches!(resolver.decide_from_snapshot(missing), FocusDecision::NoTarget { reason } if reason.contains("focus_identity_missing"))
        );
    }
    fn two_games(child_of_first: bool) -> FocusSnapshot {
        let mut first = resolver_test_process(10, "Game.exe", TaskClass::Game);
        first.starttime_ticks = Some(100);
        let mut second = resolver_test_process(20, "OtherGame.exe", TaskClass::Game);
        second.starttime_ticks = Some(200);
        if child_of_first {
            second.ppid = 10;
        }
        resolver_test_snapshot(
            vec![resolver_test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 20],
                Some(10),
                0.95,
                0.95,
            )],
            vec![first, second],
            1000,
        )
    }
    #[test]
    fn sibling_games_folded_into_one_group_are_still_ambiguous() {
        let mut resolver = FocusResolver::new(resolver_test_policy()).bounded_autotune(0.85);
        assert!(
            matches!(resolver.decide_from_snapshot(two_games(false)), FocusDecision::NoTarget { reason } if reason.contains("ambiguous_game_roots"))
        );
    }
    #[test]
    fn a_game_helper_below_the_root_is_not_a_second_game() {
        let mut resolver = FocusResolver::new(resolver_test_policy()).bounded_autotune(0.85);
        assert!(
            !matches!(resolver.decide_from_snapshot(two_games(true)), FocusDecision::NoTarget { reason } if reason.contains("ambiguous_game_roots"))
        );
    }
    #[test]
    fn oscillating_winner_never_completes_required_polls() {
        let mut resolver = FocusResolver::new(resolver_test_policy()).bounded_autotune(0.85);
        for n in 1..10 {
            let pid = if n % 2 == 0 { 10 } else { 20 };
            assert!(matches!(
                resolver.decide_from_snapshot(snapshot(pid, 100 + u64::from(pid), n * 1000)),
                FocusDecision::NoTarget { .. }
            ));
        }
    }
}
