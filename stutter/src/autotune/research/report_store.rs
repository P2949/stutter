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

pub fn write_new(dir: &File, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(bytes.len() as u64 <= MAX_FILE, "record too large");
    let mut file = crate::syscall::create_new_at(dir.as_fd(), name)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    dir.sync_all()?;
    Ok(())
}

fn read_private(dir: &File, name: &str, uid: u32) -> anyhow::Result<Vec<u8>> {
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
        let index = name
            .strip_suffix(".json")
            .filter(|stem| stem.len() == 4)
            .and_then(|stem| stem.parse::<u32>().ok())
            .ok_or_else(|| anyhow::anyhow!("unexpected evidence entry"))?;
        let item: AttemptEvidenceV1 = serde_json::from_slice(&read_private(dir, &name, uid)?)?;
        anyhow::ensure!(item.slot_index == index, "evidence filename mismatch");
        found.insert(index, item);
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
        "report does not match an independent rebuild"
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
