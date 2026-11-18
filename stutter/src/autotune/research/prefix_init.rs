//! The bounded, journalled stage that initializes a research arm's Wine prefix (PLAN section 6.1).
//! A cold prefix has no `UserSettings.json`, and the screen applies a graphics profile before it
//! launches, so a cold arm can never reach its first launch through the screen. Initialization is
//! therefore its own stage, run before any profile or comparison: one supervised launch whose only
//! job is to let the game create its prefix and settings, then settle, then be verified. No profile
//! is applied and nothing is measured during it. Prefix initialization does not establish the
//! temperature of a shader cache.
//!
//! This module is the contract and its verifier: the plan (which arms need it), the state machine
//! (`Planned -> Launched -> Settled -> Verified`, or `Abandoned`), a hash-chained journal of
//! immutable records in the study directory, and the check that an arm's prefix is initialized by a
//! complete, ordered, unforged chain whose final observation shows the files it exists to create. The
//! launch itself is a live action and is not performed here.

use std::{collections::BTreeSet, fs::File, os::fd::AsRawFd, path::Path};

use serde::{Deserialize, Serialize};

use super::{
    ProtonSource,
    prefix_identity::{PrefixClass, observe},
    report_store::{read_private, write_new},
};

/// An initialization launch that has run longer than this has failed; it is never extended.
pub const MAX_INIT_SECONDS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitStage {
    Planned,
    Launched,
    /// The launched process tree is proven gone.
    Settled,
    /// The prefix now has what initialization exists to create.
    Verified,
    /// Initialization was given up; the prefix stays uninitialized and the arm blocked.
    Abandoned,
}

impl InitStage {
    pub fn permits(self, next: Self) -> bool {
        use InitStage::*;
        matches!(
            (self, next),
            (Planned, Launched)
                | (Planned, Abandoned)
                | (Launched, Settled)
                | (Launched, Abandoned)
                | (Settled, Verified)
                | (Settled, Abandoned)
        )
    }
}

/// What a plan says about one arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitNeed {
    /// A cold prefix: it must be initialized before any profile or comparison.
    Initialize,
    /// Already initialized (warm): nothing to do.
    NotNeeded,
    /// Partly initialized or unreadable: neither initializable nor comparable by this stage.
    Indeterminate,
}

/// What each arm needs, from its compat directory as it is now (read-only).
pub fn plan(arms: &[(ProtonSource, &Path)]) -> Vec<(ProtonSource, InitNeed)> {
    arms.iter()
        .map(|(source, compat)| {
            (
                *source,
                match observe(compat).class {
                    PrefixClass::Cold => InitNeed::Initialize,
                    PrefixClass::Warm => InitNeed::NotNeeded,
                    PrefixClass::Indeterminate => InitNeed::Indeterminate,
                },
            )
        })
        .collect()
}

/// One link of an arm's initialization journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitRecord {
    pub schema: u32,
    pub arm: ProtonSource,
    pub sequence: u32,
    pub stage: InitStage,
    pub unix_seconds: u64,
    /// SHA-256 of the previous record's bytes (64 zeros for the first).
    pub previous_sha256: String,
    /// What the prefix looked like when this record was made.
    pub prefix_class: String,
    pub settings_present: bool,
}

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn file_name(arm: ProtonSource, sequence: u32) -> String {
    format!("prefix-init-{}-{sequence:04}.json", arm.dir_name())
}

/// The arm's journal: its records in order, read and checked.
pub struct InitJournal<'a> {
    dir: &'a File,
    arm: ProtonSource,
    uid: u32,
    records: Vec<(InitRecord, String)>,
}

impl<'a> InitJournal<'a> {
    /// Read the chain: gapless from one, each record naming the digest of the one before, every
    /// transition legal, times not going backwards. Anything else is an error, not a short chain.
    pub fn open(dir: &'a File, arm: ProtonSource, uid: u32) -> anyhow::Result<Self> {
        let mut records = Vec::new();
        let mut previous = ZERO.to_owned();
        // Enumerate the entire namespace first: checking only the next filename mistakes arbitrary
        // gaps and inaccessible records for the end of the journal.
        let prefix = format!("prefix-init-{}-", arm.dir_name());
        let mut sequences = BTreeSet::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 initialization journal entry"))?;
            if !name.starts_with(&prefix) {
                continue;
            }
            let sequence = name
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(".json"))
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| anyhow::anyhow!("invalid initialization journal filename"))?;
            anyhow::ensure!(
                sequence > 0 && sequence <= 16 && name == file_name(arm, sequence),
                "invalid initialization journal filename"
            );
            sequences.insert(sequence);
        }
        for (index, sequence) in sequences.into_iter().enumerate() {
            anyhow::ensure!(
                sequence as usize == index + 1,
                "the initialization journal has a gap"
            );
            let bytes = read_private(dir, &file_name(arm, sequence), uid)?;
            let record: InitRecord = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                record.schema == 1
                    && record.arm == arm
                    && record.sequence == sequence
                    && record.previous_sha256 == previous,
                "the initialization journal is not a chain"
            );
            match records.last() {
                None => anyhow::ensure!(
                    record.stage == InitStage::Planned,
                    "a journal starts planned"
                ),
                Some((last, _)) => {
                    let last: &InitRecord = last;
                    anyhow::ensure!(
                        last.stage.permits(record.stage)
                            && record.unix_seconds >= last.unix_seconds,
                        "an illegal initialization transition"
                    );
                }
            }
            previous = stutter_core::hash::sha256_hex(&bytes);
            records.push((record, previous.clone()));
        }
        Ok(Self {
            dir,
            arm,
            uid,
            records,
        })
    }

    pub fn stage(&self) -> Option<InitStage> {
        self.records.last().map(|(record, _)| record.stage)
    }

    /// Append the next stage, observing the prefix now. A transition the machine does not permit,
    /// one that would outlast `MAX_INIT_SECONDS` since the launch, or `Verified` over a prefix that
    /// does not have its settings and registry is refused.
    pub fn advance(
        &mut self,
        next: InitStage,
        compat_dir: &Path,
        settings: &Path,
        now: u64,
    ) -> anyhow::Result<()> {
        let last = self.records.last();
        let allowed = match last {
            None => next == InitStage::Planned,
            Some((record, _)) => record.stage.permits(next),
        };
        anyhow::ensure!(allowed, "an illegal initialization transition");
        // A record that goes back in time would write a chain `open` then refuses to read.
        anyhow::ensure!(
            last.is_none_or(|(record, _)| now >= record.unix_seconds),
            "initialization time went backwards"
        );
        let launched_at = self
            .records
            .iter()
            .find(|(record, _)| record.stage == InitStage::Launched)
            .map(|(record, _)| record.unix_seconds);
        if matches!(next, InitStage::Settled | InitStage::Verified)
            && let Some(launched) = launched_at
        {
            anyhow::ensure!(
                now.saturating_sub(launched) <= MAX_INIT_SECONDS,
                "the initialization outlasted its bound"
            );
        }
        let observed = observe(compat_dir);
        let settings_present = std::fs::symlink_metadata(settings).is_ok_and(|m| m.is_file());
        if next == InitStage::Verified {
            anyhow::ensure!(
                observed.class == PrefixClass::Warm && settings_present,
                "the prefix does not hold what initialization creates"
            );
        }
        let record = InitRecord {
            schema: 1,
            arm: self.arm,
            sequence: self.records.len() as u32 + 1,
            stage: next,
            unix_seconds: now,
            previous_sha256: last.map_or_else(|| ZERO.to_owned(), |(_, digest)| digest.clone()),
            prefix_class: observed.class.label().to_owned(),
            settings_present,
        };
        let bytes = serde_json::to_vec(&record)?;
        write_new(self.dir, &file_name(self.arm, record.sequence), &bytes)?;
        let digest = stutter_core::hash::sha256_hex(&bytes);
        // Re-read what was written: the journal is what is on disk, not what was meant.
        anyhow::ensure!(
            read_private(self.dir, &file_name(self.arm, record.sequence), self.uid)? == bytes,
            "the initialization record did not persist"
        );
        self.records.push((record, digest));
        Ok(())
    }

    /// Whether the chain ends verified: the arm's prefix was initialized by this stage.
    pub fn verified(&self) -> bool {
        self.stage() == Some(InitStage::Verified)
    }

    /// Whether a graphics profile may be applied to the arm now: never while a launch is in flight
    /// or unsettled, and never before initialization is verified for a cold arm.
    pub fn profile_allowed(&self) -> bool {
        matches!(self.stage(), None | Some(InitStage::Verified))
    }
}

/// Whether an arm is initialized for the study: its prefix is warm and either it was never ours to
/// initialize (no journal) or its journal ends verified. A journal that does not read as a chain, or
/// that ends anywhere else, is not initialized.
pub fn initialized(
    dir: &File,
    arm: ProtonSource,
    compat_dir: &Path,
    uid: u32,
) -> anyhow::Result<bool> {
    let journal = InitJournal::open(dir, arm, uid)?;
    Ok(observe(compat_dir).class == PrefixClass::Warm
        && (journal.stage().is_none() || journal.verified()))
}

#[cfg(test)]
#[path = "prefix_init_tests.rs"]
mod tests;
