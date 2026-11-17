//! A read-only inspection of a campaign's lease directory (PLAN G1: a finite, digest-bound intent held
//! by one resident lease, with a safe journal). It takes no lock, creates nothing and changes
//! nothing: it re-reads the sealed records and says whether they hold together. The lease writes an
//! `intent.json` header (the intent and its digest) and one `event-NNNN.json` per phase transition,
//! each naming the SHA-256 of the record before it, so a gap, a reordering, an edit or an impossible
//! transition is visible without trusting the controller that wrote them.

use std::{fs::File, path::Path};

use serde::Deserialize;

use super::{ResearchIntentV1, ResearchPhase, report_store::read_private};

/// A campaign has a bounded number of transitions; more is not a lease.
const MAX_EVENTS: usize = 4096;

#[derive(Deserialize)]
struct Event {
    sequence: u32,
    from: ResearchPhase,
    to: ResearchPhase,
    previous_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LeaseInspection {
    pub events: u32,
    /// Every record names the SHA-256 of the one before it (the header for the first) and the
    /// sequence is gapless from one.
    pub chain_valid: bool,
    /// Every transition is one the state machine permits and each starts where the last ended.
    pub transitions_valid: bool,
    /// The header's digest is the digest of the intent it carries.
    pub header_matches_intent: bool,
    /// The intent digest the lease was created for.
    pub intent_sha256: String,
    pub final_phase: ResearchPhase,
    /// The campaign ended (completed, blocked or aborted) rather than stopping mid-way.
    pub terminal: bool,
}

fn event_name(sequence: usize) -> String {
    format!("event-{sequence:04}.json")
}

/// Inspect the lease directory of one campaign.
pub fn inspect(dir: &Path, uid: u32) -> anyhow::Result<LeaseInspection> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(dir)?;
    anyhow::ensure!(
        meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "the lease directory must be private and owned by this user"
    );
    let handle = File::open(dir)?;
    let header_bytes = read_private(&handle, "intent.json", uid)?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)?;
    let claimed = header["intent_sha256"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the lease header has no intent digest"))?
        .to_owned();
    let intent: ResearchIntentV1 = serde_json::from_value(header["intent"].clone())?;
    let header_matches_intent = intent.digest()? == claimed;

    let mut previous = stutter_core::hash::sha256_hex(&header_bytes);
    let mut phase = ResearchPhase::Requested;
    let (mut chain_valid, mut transitions_valid, mut count) = (true, true, 0u32);
    // Every event record present, by sequence, read from the directory itself rather than probed one
    // name at a time: a gap, a record past the end or an `event-` entry that is not a well-formed
    // record cannot hide behind a read that happened to fail.
    let mut present = std::collections::BTreeSet::new();
    let mut stray = false;
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event-") {
            continue;
        }
        match name
            .strip_prefix("event-")
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|digits| digits.parse::<usize>().ok())
        {
            Some(sequence)
                if (1..=MAX_EVENTS).contains(&sequence) && event_name(sequence) == name =>
            {
                present.insert(sequence);
            }
            _ => stray = true,
        }
    }
    let contiguous = (1..=MAX_EVENTS)
        .take_while(|sequence| present.contains(sequence))
        .count();
    chain_valid &= !stray && present.len() == contiguous;
    for sequence in 1..=contiguous {
        // A record that is present but cannot be read is an error, not the end of the chain.
        let bytes = read_private(&handle, &event_name(sequence), uid)?;
        let event: Event = serde_json::from_slice(&bytes)?;
        chain_valid &= event.sequence as usize == sequence && event.previous_sha256 == previous;
        transitions_valid &= event.from == phase && event.from.permits(event.to);
        previous = stutter_core::hash::sha256_hex(&bytes);
        phase = event.to;
        count += 1;
    }
    Ok(LeaseInspection {
        events: count,
        chain_valid,
        transitions_valid,
        header_matches_intent,
        intent_sha256: claimed,
        final_phase: phase,
        terminal: matches!(
            phase,
            ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
        ),
    })
}

#[cfg(test)]
#[path = "lease_inspect_tests.rs"]
mod tests;
