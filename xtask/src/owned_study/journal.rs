//! The study's strict, crash-aware record store.
//!
//! One verification function serves resume and analysis. A new study commits each slot as its own
//! file: a `.started` marker is created (and synced) before any process launches, and the result is
//! written to a temporary file, synced, renamed to `<slot>.json` and the directory synced. A slot
//! that has a marker but no committed record is AMBIGUOUS and is never silently rerun. Nothing in an
//! existing study is ever skipped because it failed to parse: any unreadable, torn, duplicate,
//! unknown, mismatched or out-of-order record is an error.
use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::manifest::{Manifest, Slot};
use crate::evidence_bundle::sha256::sha256_hex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub study_id: String,
    pub manifest_sha256: String,
    pub slot: String,
    pub block: u32,
    pub arm: String,
    pub valid: bool,
    pub reason: Option<String>,
    pub elapsed_ns: Option<u64>,
    /// Digest of the kernel, boot, effective CPUs, sched_ext state, source tree and work identity.
    /// Required for every record of a new study; absent only in legacy (pre-context) studies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// sha256 of the study's output artifact after this slot's build, when the manifest names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_sha256: Option<String>,
}

#[derive(Debug)]
pub enum StudyReadError {
    Unreadable(PathBuf, io::Error),
    Malformed(String),
    Mismatch(String),
    Unknown(String),
    OutOfOrder(String),
    Ambiguous(String),
    Context(String),
}

impl fmt::Display for StudyReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(path, error) => write!(f, "cannot read {}: {error}", path.display()),
            Self::Malformed(what) => write!(f, "malformed study record: {what}"),
            Self::Mismatch(what) => write!(f, "study record does not match the manifest: {what}"),
            Self::Unknown(what) => write!(f, "unknown study entry: {what}"),
            Self::OutOfOrder(what) => write!(f, "study records are out of order: {what}"),
            Self::Ambiguous(what) => write!(
                f,
                "slot {what} was started but never committed (UnsafeToResume): start a new study id"
            ),
            Self::Context(what) => write!(f, "study context problem: {what}"),
        }
    }
}

impl std::error::Error for StudyReadError {}

/// What the records are bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Resuming: every record must carry exactly this context.
    Resume(String),
    /// Analysing: records must carry one common context, or (legacy) none at all.
    Analyze,
}

#[derive(Debug)]
pub struct Verified {
    pub records: Vec<Record>,
    /// The slots not yet recorded, in schedule order (the recorded slots are a schedule prefix).
    pub pending: Vec<Slot>,
    pub legacy_without_context: usize,
    pub context: Option<String>,
}

/// The one verification used by resume and analysis.
pub fn verify_records(
    manifest: &Manifest,
    records: Vec<Record>,
    mode: &Mode,
) -> Result<Verified, StudyReadError> {
    let digest = manifest.digest();
    let schedule = manifest.schedule();
    let mut seen = BTreeSet::new();
    if records.len() > schedule.len() {
        return Err(StudyReadError::Unknown("more records than slots".into()));
    }
    for (index, record) in records.iter().enumerate() {
        if record.study_id != manifest.study_id || record.manifest_sha256 != digest {
            return Err(StudyReadError::Mismatch(format!(
                "{} belongs to another study or manifest",
                record.slot
            )));
        }
        let Some(position) = schedule.iter().position(|slot| slot.id == record.slot) else {
            return Err(StudyReadError::Unknown(format!("slot {}", record.slot)));
        };
        if !seen.insert(record.slot.as_str()) {
            return Err(StudyReadError::Mismatch(format!(
                "duplicate slot {}",
                record.slot
            )));
        }
        let expected = &schedule[position];
        let arm = &manifest.arms()[expected.arm].name;
        if record.block != expected.block || &record.arm != arm {
            return Err(StudyReadError::Mismatch(format!(
                "{} has another block or arm than the schedule",
                record.slot
            )));
        }
        if position != index {
            return Err(StudyReadError::OutOfOrder(record.slot.clone()));
        }
        let consistent = if record.valid {
            record.elapsed_ns.is_some() && record.reason.is_none()
        } else {
            record.elapsed_ns.is_none() && record.reason.is_some()
        };
        if !consistent {
            return Err(StudyReadError::Malformed(format!(
                "{} has inconsistent validity fields",
                record.slot
            )));
        }
    }
    let outputs: BTreeSet<&str> = records
        .iter()
        .filter(|r| r.valid)
        .filter_map(|r| r.output_sha256.as_deref())
        .collect();
    if manifest.output_artifact.is_some() {
        if records.iter().any(|r| r.valid && r.output_sha256.is_none()) {
            return Err(StudyReadError::Mismatch(
                "a valid slot has no output hash".into(),
            ));
        }
        if outputs.len() > 1 {
            return Err(StudyReadError::Mismatch(
                "valid slots built different outputs: the arms did not do the same work".into(),
            ));
        }
    } else if records.iter().any(|r| r.output_sha256.is_some()) {
        return Err(StudyReadError::Mismatch(
            "an output hash without a configured output artifact".into(),
        ));
    }
    let with: BTreeSet<&str> = records
        .iter()
        .filter_map(|r| r.context.as_deref())
        .collect();
    let without = records.iter().filter(|r| r.context.is_none()).count();
    if with.len() > 1 || (!with.is_empty() && without > 0) {
        return Err(StudyReadError::Context(
            "records span more than one context, or mix recorded and unrecorded contexts".into(),
        ));
    }
    if let Mode::Resume(current) = mode
        && (without > 0 || with.iter().any(|context| context != current))
    {
        return Err(StudyReadError::Context(
            "the context changed since a slot was recorded (or a record has none)".into(),
        ));
    }
    let context = with.iter().next().map(|c| (*c).to_owned());
    let pending = schedule[records.len()..].to_vec();
    Ok(Verified {
        legacy_without_context: without,
        context,
        pending,
        records,
    })
}

/// Legacy `results.jsonl`: every line must parse; a missing newline at the end is a torn tail.
pub fn read_jsonl(path: &Path) -> Result<Vec<Record>, StudyReadError> {
    let bytes =
        fs::read(path).map_err(|error| StudyReadError::Unreadable(path.to_owned(), error))?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.last() != Some(&b'\n') {
        return Err(StudyReadError::Malformed("truncated final line".into()));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| StudyReadError::Malformed("not UTF-8".into()))?;
    text.lines()
        .enumerate()
        .map(|(number, line)| {
            serde_json::from_str(line)
                .map_err(|error| StudyReadError::Malformed(format!("line {}: {error}", number + 1)))
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Committed {
    record: Record,
    record_sha256: String,
}

fn sync_directory(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Marks a slot as started, durably, before any process of it launches.
pub fn begin_slot(slots: &Path, id: &str) -> io::Result<()> {
    let marker = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(slots.join(format!("{id}.started")))?;
    marker.sync_all()?;
    sync_directory(slots)
}

/// Atomically and durably commits a slot's record.
pub fn commit_slot(slots: &Path, record: &Record) -> io::Result<()> {
    let body = Committed {
        record: record.clone(),
        record_sha256: sha256_hex(&serde_json::to_vec(record).map_err(io::Error::other)?),
    };
    let temporary = slots.join(format!("{}.json.tmp", record.slot));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(&body).map_err(io::Error::other)?)?;
    file.sync_all()?;
    fs::rename(&temporary, slots.join(format!("{}.json", record.slot)))?;
    sync_directory(slots)
}

/// Reads a slot directory: committed slots must be a contiguous schedule prefix; a started-but-not-
/// committed slot, a leftover temporary file, a corrupted record or any unknown entry is an error.
pub fn read_slots(slots: &Path, manifest: &Manifest) -> Result<Vec<Record>, StudyReadError> {
    let schedule = manifest.schedule();
    let known: BTreeSet<String> = schedule
        .iter()
        .flat_map(|s| [format!("{}.json", s.id), format!("{}.started", s.id)])
        .collect();
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(slots).map_err(|e| StudyReadError::Unreadable(slots.to_owned(), e))? {
        let entry = entry.map_err(|e| StudyReadError::Unreadable(slots.to_owned(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json.tmp") {
            return Err(StudyReadError::Ambiguous(
                name.trim_end_matches(".json.tmp").to_owned(),
            ));
        }
        if !known.contains(&name) {
            return Err(StudyReadError::Unknown(name));
        }
        names.insert(name);
    }
    let mut records = Vec::new();
    let mut gap: Option<&str> = None;
    for slot in &schedule {
        let committed = names.contains(&format!("{}.json", slot.id));
        let started = names.contains(&format!("{}.started", slot.id));
        match (committed, started) {
            (true, _) if gap.is_some() => return Err(StudyReadError::OutOfOrder(slot.id.clone())),
            (true, _) => {
                let path = slots.join(format!("{}.json", slot.id));
                let bytes =
                    fs::read(&path).map_err(|e| StudyReadError::Unreadable(path.clone(), e))?;
                let body: Committed = serde_json::from_slice(&bytes)
                    .map_err(|e| StudyReadError::Malformed(format!("{}: {e}", slot.id)))?;
                let digest = sha256_hex(&serde_json::to_vec(&body.record).unwrap_or_default());
                if digest != body.record_sha256 {
                    return Err(StudyReadError::Malformed(format!(
                        "{}: record digest mismatch",
                        slot.id
                    )));
                }
                records.push(body.record);
            }
            (false, true) => return Err(StudyReadError::Ambiguous(slot.id.clone())),
            (false, false) => gap = gap.or(Some(&slot.id)),
        }
    }
    Ok(records)
}

/// Reads and verifies a study directory: the slot journal when present, otherwise legacy
/// `results.jsonl`. A missing store is an error unless the caller says a new study may start.
pub fn load(
    dir: &Path,
    manifest: &Manifest,
    mode: &Mode,
    new_ok: bool,
) -> Result<Verified, StudyReadError> {
    let slots = dir.join("slots");
    let jsonl = dir.join("results.jsonl");
    let records = match (fs::metadata(&slots), fs::metadata(&jsonl)) {
        (Ok(meta), _) if meta.is_dir() => read_slots(&slots, manifest)?,
        (Ok(_), _) => return Err(StudyReadError::Unknown("slots is not a directory".into())),
        (Err(e), _) if e.kind() != io::ErrorKind::NotFound => {
            return Err(StudyReadError::Unreadable(slots, e));
        }
        (Err(_), Ok(_)) => read_jsonl(&jsonl)?,
        (Err(_), Err(e)) if e.kind() == io::ErrorKind::NotFound && new_ok => Vec::new(),
        (Err(_), Err(e)) => return Err(StudyReadError::Unreadable(jsonl, e)),
    };
    verify_records(manifest, records, mode)
}
