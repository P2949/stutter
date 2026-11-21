//! On-disk layout of one campaign's raw evidence and report: `intent.json`, `evidence/NNNN.json`,
//! `report.json` beside the `journal/` directory. Everything is private, user-owned and bounded.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
};

use super::{
    AttemptSlot, ResearchIntentV1, ResearchJournal,
    driver::build_schedule,
    report::{AttemptEvidenceV1, ResearchReportV1, build_report},
};

const MAX_FILE: u64 = 64 * 1024;

/// Canonical campaign-scoped baseline name; the full intent digest prevents cross-campaign reuse.
pub(crate) fn baseline_name(kind: &str, campaign: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        campaign.len() == 64
            && campaign
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid campaign identity"
    );
    anyhow::ensure!(
        matches!(kind, "host-hardware" | "protected-state"),
        "invalid baseline kind"
    );
    Ok(format!("{kind}-before-{campaign}.json"))
}

/// Publish a new record atomically: the bytes are written and synchronized under a temporary name,
/// then renamed into place without replacing anything, so a crash never leaves a partial canonical
/// file and an existing record is never overwritten (`AlreadyExists`).
pub fn write_new(dir: &File, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    write_new_bounded(dir, name, bytes, MAX_FILE)
}

/// As `write_new` for a larger record (a raw native file), with the caller's own size bound.
pub(crate) fn write_new_bounded(
    dir: &File,
    name: &str,
    bytes: &[u8],
    max: u64,
) -> anyhow::Result<()> {
    anyhow::ensure!(bytes.len() as u64 <= max, "record too large");
    anyhow::ensure!(!name.starts_with('.'), "record name is reserved");
    let tmp = format!(".{name}.tmp");
    let tmp_path = format!("/proc/self/fd/{}/{tmp}", dir.as_raw_fd());
    // A temporary file left by an interrupted write was never canonical; discard it.
    match std::fs::remove_file(&tmp_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut file = crate::syscall::create_new_at(dir.as_fd(), &tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = crate::syscall::rename_noreplace_at(dir.as_fd(), &tmp, name) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error.into());
    }
    dir.sync_all()?;
    Ok(())
}

pub(crate) fn read_private(dir: &File, name: &str, uid: u32) -> anyhow::Result<Vec<u8>> {
    let file = crate::syscall::openat_nofollow(dir.as_fd(), name)?;
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file()
            && meta.uid() == uid
            && meta.nlink() == 1
            && meta.mode() & 0o077 == 0
            && meta.len() <= MAX_FILE,
        "untrusted campaign record"
    );
    let mut bytes = Vec::new();
    (&file).take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == meta.len(),
        "campaign record changed while reading"
    );
    Ok(bytes)
}

/// Persist one attempt's raw evidence and return the digest its journal seal must commit to.
pub fn write_evidence(
    dir: &File,
    slot: &AttemptSlot,
    item: &AttemptEvidenceV1,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        item.schema_version == 1
            && item.slot_index == slot.index
            && item.average_fps.is_finite()
            && item.average_fps > 0.0,
        "invalid attempt evidence"
    );
    write_new(
        dir,
        &format!("{:04}.json", slot.index),
        &serde_json::to_vec(item)?,
    )?;
    item.digest()
}

pub fn read_evidence(dir: &File, uid: u32) -> anyhow::Result<BTreeMap<u32, AttemptEvidenceV1>> {
    let mut found = BTreeMap::new();
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 evidence entry"))?;
        if name.starts_with('.') && name.ends_with(".json.tmp") {
            continue; // an interrupted write that never became canonical
        }
        let index = name
            .strip_suffix(".json")
            .filter(|stem| stem.len() == 4)
            .and_then(|stem| stem.parse::<u32>().ok())
            .ok_or_else(|| anyhow::anyhow!("unexpected evidence entry"))?;
        anyhow::ensure!(
            name == format!("{index:04}.json"),
            "noncanonical evidence filename"
        );
        let item: AttemptEvidenceV1 = serde_json::from_slice(&read_private(dir, &name, uid)?)?;
        anyhow::ensure!(item.slot_index == index, "evidence filename mismatch");
        anyhow::ensure!(
            found.insert(index, item).is_none() && found.len() <= 1000,
            "duplicate or excessive evidence"
        );
    }
    Ok(found)
}

pub fn write_report(campaign: &File, report: &ResearchReportV1) -> anyhow::Result<()> {
    write_new(campaign, "report.json", &serde_json::to_vec(report)?)
}

/// Rebuild the report from the raw journal and evidence and require it to equal `report.json`.
pub fn load_and_verify(campaign: &File, uid: u32) -> anyhow::Result<ResearchReportV1> {
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&read_private(campaign, "intent.json", uid)?)?;
    intent.validate()?;
    anyhow::ensure!(intent.uid == uid, "campaign belongs to another user");
    // Compare raw bytes: parsing floats back is not guaranteed bit-exact.
    let stored_bytes = read_private(campaign, "report.json", uid)?;
    let stored: ResearchReportV1 = serde_json::from_slice(&stored_bytes)?;
    let journal_dir = crate::syscall::openat_dir_nofollow(campaign.as_fd(), "journal")?;
    let journal = ResearchJournal::open(journal_dir, uid, build_schedule(&intent)?)?;
    let records = journal.verify()?;
    let evidence_dir = File::from(crate::syscall::openat_dir_nofollow(
        campaign.as_fd(),
        "evidence",
    )?);
    let evidence = read_evidence(&evidence_dir, uid)?;
    let rebuilt = build_report(&intent, &records, &evidence)?;
    anyhow::ensure!(
        serde_json::to_vec(&rebuilt)? == stored_bytes,
        "report does not match deterministic reconstruction"
    );
    Ok(stored)
}

/// Build the report from the journal and evidence directories (used when the campaign finishes).
pub fn build_from_dirs(
    intent: &ResearchIntentV1,
    journal: &ResearchJournal,
    evidence_dir: &File,
) -> anyhow::Result<ResearchReportV1> {
    let records = journal.verify()?;
    let evidence = read_evidence(evidence_dir, intent.uid)?;
    build_report(intent, &records, &evidence)
}

/// The bytes one attempt's evidence accounts for: its own record plus the native raw files it
/// commits to by hash (the raw files stay in the study prefix; they are still evidence).
pub fn evidence_cost(item: &AttemptEvidenceV1) -> anyhow::Result<u64> {
    let record = serde_json::to_vec(item)?.len() as u64;
    let mut raw: u64 = 0;
    if let Some(completion) = &item.native_completion {
        for file in &completion.files {
            raw = raw
                .checked_add(file.size_bytes)
                .ok_or_else(|| anyhow::anyhow!("native file sizes overflow the evidence budget"))?;
        }
    }
    record
        .checked_add(raw)
        .ok_or_else(|| anyhow::anyhow!("evidence cost overflows the evidence budget"))
}

/// A valid evidence set would exceed the frozen byte limit.
#[derive(Debug)]
pub struct EvidenceBudgetExceeded {
    pub total: u64,
    pub limit: u64,
}

impl std::fmt::Display for EvidenceBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "campaign evidence would use {} bytes, over the {} byte budget",
            self.total, self.limit
        )
    }
}

impl std::error::Error for EvidenceBudgetExceeded {}

/// Refuse evidence that would take the campaign over `max_evidence_bytes`. The cost of every
/// evidence record already written counts, so the limit is campaign-wide, not per attempt.
pub fn ensure_evidence_budget(
    dir: &File,
    uid: u32,
    max_evidence_bytes: u64,
    item: &AttemptEvidenceV1,
) -> anyhow::Result<()> {
    let mut total = evidence_cost(item)?;
    for (index, existing) in read_evidence(dir, uid)? {
        if index != item.slot_index {
            total = total
                .checked_add(evidence_cost(&existing)?)
                .ok_or_else(|| anyhow::anyhow!("campaign evidence cost overflows"))?;
        }
    }
    if total > max_evidence_bytes {
        return Err(EvidenceBudgetExceeded {
            total,
            limit: max_evidence_bytes,
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn item(index: u32) -> AttemptEvidenceV1 {
        AttemptEvidenceV1 {
            thermal: None,
            long_session: None,
            schema_version: 1,
            slot_index: index,
            average_fps: 60.0,
            native_completion: None,
        }
    }

    fn private_dir() -> (tempfile::TempDir, File) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        (tmp, dir)
    }

    #[test]
    fn a_record_is_published_atomically_and_never_replaced() {
        let (tmp, dir) = private_dir();
        write_new(&dir, "0001.json", b"first").unwrap();
        let again = write_new(&dir, "0001.json", b"second").unwrap_err();
        assert_eq!(
            again.downcast_ref::<std::io::Error>().map(|e| e.kind()),
            Some(std::io::ErrorKind::AlreadyExists)
        );
        assert_eq!(
            std::fs::read(tmp.path().join("0001.json")).unwrap(),
            b"first"
        );
        // No temporary file is left behind by either outcome.
        let names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("0001.json")]);
    }

    #[test]
    fn an_interrupted_write_never_becomes_canonical_and_is_cleared_by_the_retry() {
        let (tmp, dir) = private_dir();
        // A crash after the temporary file was started leaves only the temporary name.
        std::fs::write(tmp.path().join(".0002.json.tmp"), b"{\"partial").unwrap();
        assert!(
            read_evidence(&dir, crate::syscall::geteuid())
                .unwrap()
                .is_empty()
        );
        write_new(&dir, "0002.json", b"whole").unwrap();
        assert_eq!(
            std::fs::read(tmp.path().join("0002.json")).unwrap(),
            b"whole"
        );
        assert!(!tmp.path().join(".0002.json.tmp").exists());
    }

    #[test]
    fn the_campaign_wide_evidence_budget_counts_every_record_already_written() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        let uid = crate::syscall::geteuid();
        let one = evidence_cost(&item(0)).unwrap();
        // Room for exactly two records.
        let budget = one * 2;
        ensure_evidence_budget(&dir, uid, budget, &item(0)).unwrap();
        let slot = |index| crate::autotune::research::AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index,
            arm: crate::autotune::research::ProtonSource::Ge,
        };
        write_evidence(&dir, &slot(0), &item(0)).unwrap();
        ensure_evidence_budget(&dir, uid, budget, &item(1)).unwrap();
        write_evidence(&dir, &slot(1), &item(1)).unwrap();
        assert!(ensure_evidence_budget(&dir, uid, budget, &item(2)).is_err());
        // Rewriting an existing slot is not counted twice.
        ensure_evidence_budget(&dir, uid, budget, &item(1)).unwrap();
    }
}
