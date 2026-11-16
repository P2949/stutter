//! The attested `NativeObservationSource`: it turns independently supplied game-process facts into
//! the observation the native-result adapter understands. It never reads results itself and
//! never trusts the game to describe itself: any identity drift, a second game process, or an
//! unreadable fact is `IdentityMismatch`, which seals the attempt failed.
//!
//! The production fact provider (procfs, Wine/Proton process shape, settings files) still needs
//! live qualification; this state machine is exercised with scripted facts only.

use super::{
    ProtonSource, ResearchTool,
    cyberpunk::{CyberpunkBenchmarkAttempt, CyberpunkProcessIdentity, CyberpunkScenario},
    native_probe::{NativeObservationSource, Observation},
    steam_inventory::CyberpunkInstallation,
};

/// What an independent observer sees of the Cyberpunk game right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Sighting {
    None,
    One(CyberpunkProcessIdentity),
    /// More than one game process: ownership is ambiguous.
    Multiple,
}

/// Independent facts, supplied by the production procfs/settings reader (or a test script).
pub trait GameFacts {
    fn sight(&mut self) -> anyhow::Result<Sighting>;
    /// The attempt's Proton arm was chosen: facts that live in the arm's own prefix follow it.
    fn begin_arm(&mut self, _arm: ProtonSource) {}
    /// What the sighted process has actually loaded (best effort; `None` when not observable).
    fn loaded_runtime(
        &mut self,
        _pid: u32,
    ) -> anyhow::Result<Option<super::loaded_runtime::LoadedRuntime>> {
        Ok(None)
    }
    /// Digest of the graphics/benchmark settings actually in effect for this launch.
    fn settings_sha256(&mut self) -> anyhow::Result<String>;
    fn now_unix_nanos(&mut self) -> anyhow::Result<u128>;
}

struct Seen {
    process: CyberpunkProcessIdentity,
    runtime: Option<super::loaded_runtime::LoadedRuntime>,
    settings_sha256: String,
    started_unix_nanos: u128,
}

pub struct AttestedNativeSource<F> {
    facts: F,
    installation: CyberpunkInstallation,
    /// The frozen tool set; the tool for each attempt is chosen from it by `begin(arm)`.
    tools: Vec<ResearchTool>,
    current: Option<ResearchTool>,
    owner_uid: u32,
    seen: Option<Seen>,
}

impl<F> AttestedNativeSource<F> {
    pub fn new(
        facts: F,
        installation: CyberpunkInstallation,
        tools: Vec<ResearchTool>,
        owner_uid: u32,
    ) -> Self {
        Self {
            facts,
            installation,
            tools,
            current: None,
            owner_uid,
            seen: None,
        }
    }

    fn matches_installation(&self, process: &CyberpunkProcessIdentity) -> bool {
        process.pid.as_u32() > 1
            && process.start_ticks > 0
            && process.uid == self.owner_uid
            && process.app_id == self.installation.app_id
            && process.build_id == self.installation.build_id
            && self.installation.game_executable_sha256.as_deref()
                == Some(process.executable_sha256.as_str())
    }
}

impl GameFacts for Box<dyn GameFacts + Send> {
    fn sight(&mut self) -> anyhow::Result<Sighting> {
        (**self).sight()
    }
    fn begin_arm(&mut self, arm: ProtonSource) {
        (**self).begin_arm(arm);
    }
    fn loaded_runtime(
        &mut self,
        pid: u32,
    ) -> anyhow::Result<Option<super::loaded_runtime::LoadedRuntime>> {
        (**self).loaded_runtime(pid)
    }
    fn settings_sha256(&mut self) -> anyhow::Result<String> {
        (**self).settings_sha256()
    }
    fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
        (**self).now_unix_nanos()
    }
}

impl<F: GameFacts> AttestedNativeSource<F> {
    fn observe(&mut self) -> anyhow::Result<Observation> {
        let sighting = self.facts.sight()?;
        Ok(match (sighting, self.seen.as_ref()) {
            (Sighting::Multiple, _) => Observation::IdentityMismatch,
            (Sighting::None, None) => Observation::Starting,
            (Sighting::None, Some(seen)) => {
                let proton = self
                    .current
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("no Proton tool selected for this attempt"))?;
                // The game ended after being seen: the verifier decides whether a fresh native
                // result exists for exactly this process and interval.
                Observation::Completed(Box::new(CyberpunkBenchmarkAttempt {
                    installation: self.installation.clone(),
                    process: seen.process.clone(),
                    scene: CyberpunkScenario::BuiltinBenchmark,
                    settings_sha256: seen.settings_sha256.clone(),
                    loaded_runtime: seen.runtime.clone(),
                    proton,
                    started_unix_nanos: seen.started_unix_nanos,
                    completed_unix_nanos: self.facts.now_unix_nanos()?,
                }))
            }
            (Sighting::One(process), _) if !self.matches_installation(&process) => {
                Observation::IdentityMismatch
            }
            (Sighting::One(process), Some(seen)) => {
                if process == seen.process {
                    Observation::Running
                } else {
                    // A different pid or start time is a reused or replaced process.
                    Observation::IdentityMismatch
                }
            }
            (Sighting::One(process), None) => {
                let settings_sha256 = self.facts.settings_sha256()?;
                let started_unix_nanos = self.facts.now_unix_nanos()?;
                // Best effort: a runtime that cannot be read is recorded as absent, never invented.
                let runtime = self
                    .facts
                    .loaded_runtime(process.pid.as_u32())
                    .ok()
                    .flatten();
                self.seen = Some(Seen {
                    process,
                    runtime,
                    settings_sha256,
                    started_unix_nanos,
                });
                Observation::Running
            }
        })
    }
}

impl<F: GameFacts> NativeObservationSource for AttestedNativeSource<F> {
    fn begin(&mut self, arm: ProtonSource) -> anyhow::Result<()> {
        self.seen = None;
        self.facts.begin_arm(arm);
        // An arm outside the frozen tool set can never be attributed: refuse to begin.
        self.current = None;
        self.current = Some(
            self.tools
                .iter()
                .find(|tool| tool.source == arm)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("scheduled arm is not in the frozen tool set"))?,
        );
        Ok(())
    }

    fn poll(&mut self) -> Observation {
        // An unreadable fact is never evidence that the game is absent or fine.
        self.observe().unwrap_or_else(|error| {
            log::warn!("game observation failed: {error:#}");
            Observation::IdentityMismatch
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use stutter_core::ids::Pid;

    use super::*;
    use crate::autotune::research::{ProtonSource, SteamUpdateStatus};

    const EXE: &str = "aa";

    struct Script {
        sights: VecDeque<anyhow::Result<Sighting>>,
        clock: u128,
        settings_reads: u32,
    }

    impl GameFacts for Script {
        fn sight(&mut self) -> anyhow::Result<Sighting> {
            self.sights.pop_front().unwrap_or(Ok(Sighting::None))
        }
        fn settings_sha256(&mut self) -> anyhow::Result<String> {
            self.settings_reads += 1;
            Ok("c".repeat(64))
        }
        fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
            self.clock += 1_000;
            Ok(self.clock)
        }
    }

    fn installation() -> CyberpunkInstallation {
        CyberpunkInstallation {
            app_id: 1_091_500,
            build_id: 7,
            installation_directory: "x".into(),
            manifest_update_status: SteamUpdateStatus::NoPendingUpdateIndicated,
            manifest_sha256: "d".repeat(64),
            game_executable_sha256: Some(EXE.repeat(32)),
            launcher_executable_sha256: None,
            entitlement_verified: true,
            native_benchmark_verified: false,
        }
    }

    fn process(pid: u32, ticks: u64) -> CyberpunkProcessIdentity {
        CyberpunkProcessIdentity {
            pid: Pid::new(pid),
            start_ticks: ticks,
            uid: 1000,
            app_id: 1_091_500,
            build_id: 7,
            executable_sha256: EXE.repeat(32),
        }
    }

    fn source(sights: Vec<anyhow::Result<Sighting>>) -> AttestedNativeSource<Script> {
        AttestedNativeSource::new(
            Script {
                sights: sights.into(),
                clock: 10_000,
                settings_reads: 0,
            },
            installation(),
            vec![ResearchTool {
                source: ProtonSource::ValveStable,
                identity_sha256: "b".repeat(64),
            }],
            1000,
        )
    }

    #[test]
    fn a_game_that_runs_then_exits_yields_a_completed_attempt_for_that_exact_process() {
        let one = || Ok(Sighting::One(process(500, 9)));
        let mut source = source(vec![Ok(Sighting::None), one(), one(), Ok(Sighting::None)]);
        source.begin(ProtonSource::ValveStable).unwrap();
        assert!(matches!(source.poll(), Observation::Starting));
        assert!(matches!(source.poll(), Observation::Running));
        assert!(matches!(source.poll(), Observation::Running));
        let Observation::Completed(attempt) = source.poll() else {
            panic!("expected a completed attempt")
        };
        assert_eq!(attempt.process, process(500, 9));
        assert!(attempt.started_unix_nanos < attempt.completed_unix_nanos);
        assert_eq!(attempt.settings_sha256, "c".repeat(64));
        // Settings were captured once, at first sight, not re-read each poll.
        assert_eq!(source.facts.settings_reads, 1);
    }

    #[test]
    fn identity_drift_a_second_process_or_a_foreign_build_is_a_mismatch() {
        let mut replaced = source(vec![
            Ok(Sighting::One(process(500, 9))),
            Ok(Sighting::One(process(500, 10))),
        ]);
        assert!(matches!(replaced.poll(), Observation::Running));
        assert!(matches!(replaced.poll(), Observation::IdentityMismatch));

        let mut multiple = source(vec![Ok(Sighting::Multiple)]);
        assert!(matches!(multiple.poll(), Observation::IdentityMismatch));

        for mutate in [
            (|p: &mut CyberpunkProcessIdentity| p.build_id = 8) as fn(&mut _),
            |p| p.uid = 0,
            |p| p.app_id = 570_940,
            |p| p.executable_sha256 = "bb".repeat(32),
            |p| p.start_ticks = 0,
        ] {
            let mut p = process(500, 9);
            mutate(&mut p);
            let mut foreign = source(vec![Ok(Sighting::One(p))]);
            assert!(matches!(foreign.poll(), Observation::IdentityMismatch));
        }
    }

    #[test]
    fn an_unreadable_fact_is_never_read_as_absence_or_health() {
        let mut broken = source(vec![Err(anyhow::anyhow!("procfs unreadable"))]);
        assert!(matches!(broken.poll(), Observation::IdentityMismatch));
        // A fresh attempt forgets the previous process.
        let mut again = source(vec![
            Ok(Sighting::One(process(500, 9))),
            Ok(Sighting::One(process(600, 20))),
        ]);
        assert!(matches!(again.poll(), Observation::Running));
        again.begin(ProtonSource::ValveStable).unwrap();
        assert!(matches!(again.poll(), Observation::Running));
    }

    #[test]
    fn each_attempt_is_bound_to_its_own_arms_tool_and_unknown_arms_refuse_to_begin() {
        let tools = vec![
            ResearchTool {
                source: ProtonSource::ValveStable,
                identity_sha256: "b".repeat(64),
            },
            ResearchTool {
                source: ProtonSource::Ge,
                identity_sha256: "d".repeat(64),
            },
        ];
        let one = || Ok(Sighting::One(process(500, 9)));
        let mut source = AttestedNativeSource::new(
            Script {
                sights: vec![one(), Ok(Sighting::None), one(), Ok(Sighting::None)].into(),
                clock: 10_000,
                settings_reads: 0,
            },
            installation(),
            tools,
            1000,
        );
        for (arm, expected) in [(ProtonSource::Ge, "d"), (ProtonSource::ValveStable, "b")] {
            source.begin(arm).unwrap();
            assert!(matches!(source.poll(), Observation::Running));
            let Observation::Completed(attempt) = source.poll() else {
                panic!("expected a completed attempt")
            };
            assert_eq!(attempt.proton.source, arm);
            assert_eq!(attempt.proton.identity_sha256, expected.repeat(64));
        }
        // An arm outside the frozen tool set can never be attributed.
        assert!(source.begin(ProtonSource::Em).is_err());
    }
}
