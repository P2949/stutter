//! A dossier is observed by a campaign only when the evidence it names can be reopened: the observation record lists
//! each dossier with the SHA-256 of an immutable artifact beside the study, and that artifact must hash to the
//! digest, be bound to this campaign and this dossier, name its provider and source quality, and carry a real
//! observation interval. A digest that merely looks like a digest, a missing or altered artifact, another
//! campaign's artifact or a malformed record is an integrity failure, reported as such, never an empty observation.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{crosswalk::SPECS, report_store::read_private};

/// The observation record's integrity failure: distinct from "nothing was observed".
pub const INVALID: &str = "CrosswalkObservationsInvalid";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    schema: u32,
    dossier: String,
    campaign_sha256: String,
    provider: String,
    source_quality: String,
    start_unix: u64,
    end_unix: u64,
    /// The raw window files the observation stands on: each child and the digest of its file.
    sources: Vec<Source>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    child: String,
    sha256: String,
}

/// The file name of the evidence artifact with this digest.
pub fn file_name(digest: &str) -> String {
    format!("crosswalk-evidence-{digest}.json")
}

/// A study directory's name: the first 16 hex of its study digest.
fn is_child_name(text: &str) -> bool {
    text.len() == 16
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The observation record with every row's evidence reopened and checked, or why it cannot stand. `None` (no record)
/// is simply no observations; anything else must verify in full.
pub fn verified(
    record: Option<&Value>,
    study: &std::fs::File,
    uid: u32,
    campaign: &str,
) -> Result<Value, &'static str> {
    let Some(record) = record.filter(|value| !value.is_null()) else {
        return Ok(Value::Null);
    };
    let rows = record["rows"].as_array().ok_or(INVALID)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut checked = Vec::new();
    for row in rows {
        let (Some(id), Some(digest)) = (row["id"].as_str(), row["evidence_sha256"].as_str()) else {
            return Err(INVALID);
        };
        if !SPECS.iter().any(|spec| spec.0 == id) || !is_digest(digest) || !seen.insert(id) {
            return Err(INVALID);
        }
        let bytes = read_private(study, &file_name(digest), uid).map_err(|_| INVALID)?;
        if stutter_core::hash::sha256_hex(&bytes) != digest {
            return Err(INVALID);
        }
        let artifact: Artifact = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
        if artifact.schema != 1
            || artifact.dossier != id
            || artifact.campaign_sha256 != campaign
            || artifact.provider.is_empty()
            || artifact.provider.len() > 64
            || !matches!(artifact.source_quality.as_str(), "direct" | "derived")
            || artifact.start_unix >= artifact.end_unix
            || artifact.sources.is_empty()
        {
            return Err(INVALID);
        }
        // Every raw window file the artifact names must reopen and hash to what it says: an observation stands on the
        // children's own immutable records, not on a claim about them.
        let studies =
            crate::syscall::openat_dir_nofollow(std::os::fd::AsFd::as_fd(study), "studies")
                .map(std::fs::File::from)
                .map_err(|_| INVALID)?;
        for source in &artifact.sources {
            if !is_child_name(&source.child) || !is_digest(&source.sha256) {
                return Err(INVALID);
            }
            let child = crate::syscall::openat_dir_nofollow(
                std::os::fd::AsFd::as_fd(&studies),
                &source.child,
            )
            .map(std::fs::File::from)
            .map_err(|_| INVALID)?;
            let windows = read_private(&child, super::resident_observer::WINDOWS_FILE, uid)
                .map_err(|_| INVALID)?;
            if stutter_core::hash::sha256_hex(&windows) != source.sha256 {
                return Err(INVALID);
            }
        }
        checked.push(json!({
            "id": id, "evidence_sha256": digest,
            "provider": artifact.provider, "source_quality": artifact.source_quality,
        }));
    }
    Ok(json!({ "rows": checked }))
}

/// The dossiers a verified record observed, by id with the digest of the evidence. A record the report marked
/// invalid is an error; an absent one observes nothing.
pub fn observations_from(value: Option<&Value>) -> Result<BTreeMap<String, String>, &'static str> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(BTreeMap::new());
    };
    if value.get("error").is_some() {
        return Err(INVALID);
    }
    let mut found = BTreeMap::new();
    for row in value["rows"].as_array().ok_or(INVALID)? {
        let (Some(id), Some(digest)) = (row["id"].as_str(), row["evidence_sha256"].as_str()) else {
            return Err(INVALID);
        };
        if !SPECS.iter().any(|spec| spec.0 == id)
            || !is_digest(digest)
            || found.insert(id.to_owned(), digest.to_owned()).is_some()
        {
            return Err(INVALID);
        }
    }
    Ok(found)
}

#[cfg(test)]
#[path = "crosswalk_evidence_tests.rs"]
mod tests;

/// What the report reads of the observation record: looked for in the campaign's own directory (where the producer
/// wrote it), with every row's evidence reopened. A record that cannot stand is marked, never read as no observations.
pub fn observed_in(campaign_dir: &std::fs::File, uid: u32, campaign: &str) -> Value {
    use super::newest_record::{Newest, newest_in};
    match newest_in::<Value>(
        campaign_dir,
        super::crosswalk::OBSERVATIONS_FILE.trim_end_matches(".json"),
        |_| 0,
    ) {
        Newest::Found(record) => verified(Some(&record), campaign_dir, uid, campaign)
            .unwrap_or_else(|error| serde_json::json!({ "error": error })),
        Newest::Invalid => serde_json::json!({ "error": INVALID }),
        Newest::None => Value::Null,
    }
}
