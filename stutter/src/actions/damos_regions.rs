//! Hot and cold memory regions from a scheme's `tried_regions` (package W11c).
//!
//! DAMON lists the regions a scheme tried, each with its access count and age, under
//! `schemes/N/tried_regions/<i>/`. This reads that directory with a hard bound and summarizes it
//! into totals and the hottest and coldest regions. It is a description of recent access
//! frequency, never proof that reclaiming anything is safe, and reading writes nothing.
use std::{fs, path::Path};

/// The most regions one read will take, so a huge listing cannot stall the caller.
pub const MAX_TRIED_REGIONS: usize = 1024;
/// How many hottest and coldest regions the summary names.
pub const NAMED_REGIONS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriedRegion {
    pub start: u64,
    pub end: u64,
    /// Sampled accesses in the last aggregation interval.
    pub nr_accesses: u64,
    /// Aggregation intervals the region has kept its access pattern.
    pub age: u64,
}

impl TriedRegion {
    pub fn bytes(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionSummary {
    pub regions: usize,
    /// More regions existed than `MAX_TRIED_REGIONS`; the totals describe the part read.
    pub truncated: bool,
    pub total_bytes: u64,
    /// At least `hot_threshold` accesses.
    pub hot_bytes: u64,
    /// No sampled access at all.
    pub cold_bytes: u64,
    pub hottest: Vec<TriedRegion>,
    /// Coldest by accesses, then oldest, so the longest-idle memory is named first.
    pub coldest: Vec<TriedRegion>,
}

/// Reads the listed regions. A region with a missing or unparseable field, or an empty or
/// inverted range, is skipped rather than guessed.
pub fn read_tried_regions(tried_regions: &Path) -> Option<(Vec<TriedRegion>, bool)> {
    let count = fs::read_dir(tried_regions).ok()?.flatten().count();
    let number = |dir: &Path, name: &str| -> Option<u64> {
        fs::read_to_string(dir.join(name)).ok()?.trim().parse().ok()
    };
    let mut regions = Vec::new();
    for index in 0..count.min(MAX_TRIED_REGIONS) {
        let dir = tried_regions.join(index.to_string());
        let (Some(start), Some(end), Some(nr_accesses), Some(age)) = (
            number(&dir, "start"),
            number(&dir, "end"),
            number(&dir, "nr_accesses"),
            number(&dir, "age"),
        ) else {
            continue;
        };
        if end > start {
            regions.push(TriedRegion {
                start,
                end,
                nr_accesses,
                age,
            });
        }
    }
    Some((regions, count > MAX_TRIED_REGIONS))
}

pub fn summarize(regions: &[TriedRegion], truncated: bool, hot_threshold: u64) -> RegionSummary {
    let sum = |keep: &dyn Fn(&TriedRegion) -> bool| -> u64 {
        regions
            .iter()
            .filter(|region| keep(region))
            .fold(0u64, |total, region| total.saturating_add(region.bytes()))
    };
    let mut hottest = regions.to_vec();
    hottest.sort_by(|a, b| {
        b.nr_accesses
            .cmp(&a.nr_accesses)
            .then(b.bytes().cmp(&a.bytes()))
            .then(a.start.cmp(&b.start))
    });
    hottest.truncate(NAMED_REGIONS);
    let mut coldest = regions.to_vec();
    coldest.sort_by(|a, b| {
        a.nr_accesses
            .cmp(&b.nr_accesses)
            .then(b.age.cmp(&a.age))
            .then(a.start.cmp(&b.start))
    });
    coldest.truncate(NAMED_REGIONS);
    RegionSummary {
        regions: regions.len(),
        truncated,
        total_bytes: sum(&|_| true),
        hot_bytes: sum(&|r| r.nr_accesses >= hot_threshold.max(1)),
        cold_bytes: sum(&|r| r.nr_accesses == 0),
        hottest,
        coldest,
    }
}

#[cfg(test)]
#[path = "damos_regions_tests.rs"]
mod tests;
