//! Source-bound join of resident telemetry to one attested game attempt (PLAN P4). Every sample
//! carries its producer, clock and source epoch; a sample joins a phase window only when it was
//! produced for the same game process (uid, pid, start ticks), under the epoch that was current
//! when the window opened, and inside that window. Mixed epochs, a different process, or a
//! missing identity are rejected and counted, never silently merged.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Warmup,
    Measuring,
    Washout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Runnable,
    Frames,
    Psi,
    CpuPower,
    Gpu,
    Owners,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameIdentity {
    pub uid: u32,
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub source: Source,
    pub identity: Option<GameIdentity>,
    pub epoch: u64,
    /// Monotonic nanoseconds on the sample's own recorded clock. Zero means unknown.
    pub monotonic_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reject {
    NoIdentity,
    WrongProcess,
    EpochMismatch,
    UnknownTimestamp,
    OutsideWindow,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Joined {
    pub phase: Option<Phase>,
    pub accepted: BTreeMap<Source, u64>,
    pub rejected: BTreeMap<(Source, Reject), u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub phase: Phase,
    pub start_ns: u64,
    pub end_ns: u64,
}

/// Epochs are fixed per source when a window opens; a renderer rebuild starts a new epoch and a
/// new window rather than extending this one.
pub fn join(
    game: GameIdentity,
    window: Window,
    epochs: &BTreeMap<Source, u64>,
    samples: &[Sample],
) -> Joined {
    let mut out = Joined {
        phase: Some(window.phase),
        ..Joined::default()
    };
    for sample in samples {
        let verdict = match sample.identity {
            None => Err(Reject::NoIdentity),
            Some(identity) if identity != game => Err(Reject::WrongProcess),
            Some(_) if epochs.get(&sample.source) != Some(&sample.epoch) => {
                Err(Reject::EpochMismatch)
            }
            Some(_) if sample.monotonic_ns == 0 => Err(Reject::UnknownTimestamp),
            Some(_)
                if sample.monotonic_ns < window.start_ns
                    || sample.monotonic_ns >= window.end_ns =>
            {
                Err(Reject::OutsideWindow)
            }
            Some(_) => Ok(()),
        };
        match verdict {
            Ok(()) => *out.accepted.entry(sample.source).or_default() += 1,
            Err(reason) => *out.rejected.entry((sample.source, reason)).or_default() += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: GameIdentity = GameIdentity {
        uid: 1000,
        pid: stutter_core::ids::Pid::new(500),
        start_ticks: 77,
    };

    fn sample(identity: Option<GameIdentity>, epoch: u64, ns: u64) -> Sample {
        Sample {
            source: Source::Frames,
            identity,
            epoch,
            monotonic_ns: ns,
        }
    }

    fn run(samples: &[Sample]) -> Joined {
        let window = Window {
            phase: Phase::Measuring,
            start_ns: 100,
            end_ns: 200,
        };
        join(
            GAME,
            window,
            &BTreeMap::from([(Source::Frames, 3)]),
            samples,
        )
    }

    #[test]
    fn only_same_process_same_epoch_in_window_samples_join() {
        let reused = GameIdentity {
            start_ticks: 78,
            ..GAME
        };
        let joined = run(&[
            sample(Some(GAME), 3, 100),
            sample(Some(GAME), 3, 199),
            sample(Some(GAME), 3, 200),
            sample(Some(GAME), 3, 99),
            sample(Some(GAME), 4, 150),
            sample(Some(GAME), 3, 0),
            sample(Some(reused), 3, 150),
            sample(None, 3, 150),
        ]);
        assert_eq!(joined.accepted[&Source::Frames], 2);
        let n = |r| joined.rejected[&(Source::Frames, r)];
        assert_eq!(n(Reject::OutsideWindow), 2);
        assert_eq!(n(Reject::EpochMismatch), 1);
        assert_eq!(n(Reject::UnknownTimestamp), 1);
        assert_eq!(n(Reject::WrongProcess), 1);
        assert_eq!(n(Reject::NoIdentity), 1);
    }

    #[test]
    fn a_source_without_a_declared_epoch_never_joins() {
        let window = Window {
            phase: Phase::Warmup,
            start_ns: 0,
            end_ns: u64::MAX,
        };
        let joined = join(GAME, window, &BTreeMap::new(), &[sample(Some(GAME), 3, 10)]);
        assert!(joined.accepted.is_empty());
        assert_eq!(joined.rejected[&(Source::Frames, Reject::EpochMismatch)], 1);
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn every_source_and_phase_joins_independently() {
        let game = GameIdentity {
            uid: 1,
            pid: stutter_core::ids::Pid::new(2),
            start_ticks: 3,
        };
        let sources = [
            Source::Runnable,
            Source::Frames,
            Source::Psi,
            Source::CpuPower,
            Source::Gpu,
            Source::Owners,
        ];
        let epochs: BTreeMap<_, _> = sources.iter().map(|s| (*s, 1)).collect();
        let samples: Vec<_> = sources
            .iter()
            .map(|source| Sample {
                source: *source,
                identity: Some(game),
                epoch: 1,
                monotonic_ns: 5,
            })
            .collect();
        for phase in [Phase::Warmup, Phase::Measuring, Phase::Washout] {
            let window = Window {
                phase,
                start_ns: 0,
                end_ns: 10,
            };
            let joined = join(game, window, &epochs, &samples);
            assert_eq!(joined.phase, Some(phase));
            assert_eq!(joined.accepted.len(), 6);
            assert!(joined.rejected.is_empty());
        }
    }
}
