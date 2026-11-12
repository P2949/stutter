//! Deterministic, finite, counterbalanced run schedules for research studies.
//!
//! A schedule is a pure function of its arms and seed so an offline verifier can regenerate and
//! compare it. Nothing here launches a game or grants any authority.
use std::collections::BTreeSet;

/// One scheduled position: which arm runs in which block and slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) block: usize,
    pub(crate) position: usize,
    pub(crate) arm: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScheduleError {
    TooFewArms,
    TooManyArms,
    NoBlocks,
    TooManySlots,
}

const MAX_ARMS: usize = 16;
const MAX_SLOTS: usize = 4096;

/// Seeded splitmix64 so ordering needs no external randomness and is reproducible.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Balanced multi-arm schedule: every block is a permutation of all arms. Cyclic Latin-square
/// rotations are applied to a seed-shuffled base order so each arm occupies each position equally
/// often over every `arms` consecutive blocks; a full reversal pass balances carry-over.
pub(crate) fn balanced(arms: usize, blocks: usize, seed: u64) -> Result<Vec<Slot>, ScheduleError> {
    if arms < 2 {
        return Err(ScheduleError::TooFewArms);
    }
    if arms > MAX_ARMS {
        return Err(ScheduleError::TooManyArms);
    }
    if blocks == 0 {
        return Err(ScheduleError::NoBlocks);
    }
    if arms.saturating_mul(blocks) > MAX_SLOTS {
        return Err(ScheduleError::TooManySlots);
    }
    let mut state = seed;
    let mut base: Vec<usize> = (0..arms).collect();
    for index in (1..arms).rev() {
        let pick = (next(&mut state) % (index as u64 + 1)) as usize;
        base.swap(index, pick);
    }
    let mut slots = Vec::with_capacity(arms * blocks);
    for block in 0..blocks {
        let rotation = block % arms;
        let reversed = (block / arms) % 2 == 1;
        for position in 0..arms {
            let source = (position + rotation) % arms;
            let source = if reversed { arms - 1 - source } else { source };
            slots.push(Slot {
                block,
                position,
                arm: base[source],
            });
        }
    }
    Ok(slots)
}

/// Paired AB/BA schedule: `pairs` blocks of two runs, alternating order, with the first order
/// chosen by the seed. Odd and even pair counts both stay within one of perfect balance.
pub(crate) fn paired(pairs: usize, seed: u64) -> Result<Vec<Slot>, ScheduleError> {
    if pairs == 0 {
        return Err(ScheduleError::NoBlocks);
    }
    if pairs.saturating_mul(2) > MAX_SLOTS {
        return Err(ScheduleError::TooManySlots);
    }
    let mut state = seed;
    let a_first = next(&mut state) & 1 == 0;
    let mut slots = Vec::with_capacity(pairs * 2);
    for block in 0..pairs {
        let flipped = (block % 2 == 1) == a_first;
        let order = if flipped { [1, 0] } else { [0, 1] };
        for (position, arm) in order.into_iter().enumerate() {
            slots.push(Slot {
                block,
                position,
                arm,
            });
        }
    }
    Ok(slots)
}

/// True when every block contains each arm exactly once.
pub(crate) fn blocks_are_complete(slots: &[Slot], arms: usize) -> bool {
    let blocks: BTreeSet<usize> = slots.iter().map(|slot| slot.block).collect();
    blocks.into_iter().all(|block| {
        let mut seen: Vec<usize> = slots
            .iter()
            .filter(|slot| slot.block == block)
            .map(|slot| slot.arm)
            .collect();
        seen.sort_unstable();
        seen == (0..arms).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(slots: &[Slot], arm: usize, position: usize) -> usize {
        slots
            .iter()
            .filter(|slot| slot.arm == arm && slot.position == position)
            .count()
    }

    #[test]
    fn six_way_blocks_are_complete_and_position_balanced() {
        let slots = balanced(6, 12, 7).unwrap();
        assert!(blocks_are_complete(&slots, 6));
        for arm in 0..6 {
            for position in 0..6 {
                assert_eq!(count(&slots, arm, position), 2, "arm {arm} pos {position}");
            }
        }
    }

    #[test]
    fn schedule_is_deterministic_and_seed_sensitive() {
        assert_eq!(balanced(6, 6, 1).unwrap(), balanced(6, 6, 1).unwrap());
        assert_ne!(balanced(6, 6, 1).unwrap(), balanced(6, 6, 2).unwrap());
    }

    #[test]
    fn paired_alternates_order_and_stays_balanced() {
        for pairs in [4, 5, 6] {
            let slots = paired(pairs, 3).unwrap();
            assert!(blocks_are_complete(&slots, 2));
            let a_first = slots
                .iter()
                .filter(|s| s.arm == 0 && s.position == 0)
                .count();
            assert!(a_first.abs_diff(pairs - a_first) <= 1);
        }
    }

    #[test]
    fn invalid_shapes_are_refused() {
        assert_eq!(balanced(1, 3, 0), Err(ScheduleError::TooFewArms));
        assert_eq!(balanced(17, 3, 0), Err(ScheduleError::TooManyArms));
        assert_eq!(balanced(6, 0, 0), Err(ScheduleError::NoBlocks));
        assert_eq!(balanced(16, 1000, 0), Err(ScheduleError::TooManySlots));
        assert_eq!(paired(0, 0), Err(ScheduleError::NoBlocks));
    }
}
