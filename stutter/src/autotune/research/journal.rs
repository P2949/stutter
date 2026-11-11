//! Crash-aware attempt journal. This stores evidence identities, never launch authority.
//!
//! The caller supplies an already opened, private study directory and an immutable schedule.
//! A started marker is durable before launch; an unsealed marker after restart is ambiguous.
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
};

use serde::{Deserialize, Serialize};

use super::ProtonSource;

const MAX_RECORD: u64 = 16 * 1024;
const MAX_SLOTS: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptSlot {
    pub study_sha256: String,
    pub context_sha256: String,
    pub index: u32,
    pub arm: ProtonSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptOutcome {
    Completed { evidence_sha256: String },
    Failed { code: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRecord {
    pub slot: AttemptSlot,
    pub outcome: AttemptOutcome,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seal {
    record: AttemptRecord,
    record_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Started {
    slot: AttemptSlot,
    schedule_sha256: String,
}

/// One exclusive writer to a single study's dedicated directory.
pub struct ResearchJournal {
    dir: File,
    owner_uid: u32,
    schedule: Vec<AttemptSlot>,
    schedule_sha256: String,
    active: Option<usize>,
}

fn digest_ok(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_outcome(outcome: &AttemptOutcome) -> anyhow::Result<()> {
    match outcome {
        AttemptOutcome::Completed { evidence_sha256 } => {
            anyhow::ensure!(digest_ok(evidence_sha256), "invalid evidence digest")
        }
        AttemptOutcome::Failed { code } => anyhow::ensure!(
            !code.is_empty()
                && code.len() <= 80
                && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid failure code"
        ),
    }
    Ok(())
}

impl ResearchJournal {
    /// Adopt a trusted opened directory descriptor. A second writer is refused by the kernel.
    /// The caller must establish the directory's trusted ancestry before opening it.
    pub fn open(dir: OwnedFd, owner_uid: u32, schedule: Vec<AttemptSlot>) -> anyhow::Result<Self> {
        let supplied = File::from(dir);
        let metadata = supplied.metadata()?;
        anyhow::ensure!(
            metadata.is_dir() && metadata.uid() == owner_uid && metadata.mode() & 0o077 == 0,
            "untrusted journal directory"
        );
        // Reopen the pinned descriptor to obtain a distinct open-file description. A dup of
        // a locked descriptor shares flock state and must not bypass the single-writer check.
        let dir = File::open(format!("/proc/self/fd/{}", supplied.as_raw_fd()))?;
        let reopened = dir.metadata()?;
        anyhow::ensure!(
            metadata.dev() == reopened.dev() && metadata.ino() == reopened.ino(),
            "journal directory changed"
        );
        anyhow::ensure!(
            !schedule.is_empty() && schedule.len() <= MAX_SLOTS,
            "invalid schedule size"
        );
        let first = &schedule[0];
        anyhow::ensure!(
            digest_ok(&first.study_sha256) && digest_ok(&first.context_sha256),
            "invalid schedule identity"
        );
        for (index, slot) in schedule.iter().enumerate() {
            anyhow::ensure!(
                slot.index as usize == index
                    && slot.study_sha256 == first.study_sha256
                    && slot.context_sha256 == first.context_sha256,
                "schedule is not a single ordered study"
            );
        }
        anyhow::ensure!(
            crate::syscall::flock_exclusive_nonblocking(dir.as_fd()),
            "journal already locked or locking unavailable"
        );
        let schedule_sha256 = stutter_core::hash::sha256_hex(&serde_json::to_vec(&schedule)?);
        let journal = Self {
            dir,
            owner_uid,
            schedule,
            schedule_sha256,
            active: None,
        };
        journal.scan(false)?;
        Ok(journal)
    }

    /// Validate the whole schedule prefix. An interrupted attempt always fails closed.
    pub fn verify(&self) -> anyhow::Result<Vec<AttemptRecord>> {
        self.scan(false)
    }

    /// Durably mark the next attempt before any launcher is started.
    pub fn start(&mut self) -> anyhow::Result<AttemptSlot> {
        anyhow::ensure!(self.active.is_none(), "attempt already active");
        let records = self.scan(false)?;
        let index = records.len();
        let slot = self
            .schedule
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("schedule exhausted"))?
            .clone();
        let bytes = serde_json::to_vec(&Started {
            slot: slot.clone(),
            schedule_sha256: self.schedule_sha256.clone(),
        })?;
        self.create_synced(&format!("{index:04}.started"), &bytes)?;
        self.dir.sync_all()?;
        self.active = Some(index);
        Ok(slot)
    }

    /// Atomically seal the active attempt without replacing an existing result.
    pub fn seal(&mut self, record: AttemptRecord) -> anyhow::Result<()> {
        let index = self
            .active
            .ok_or_else(|| anyhow::anyhow!("no in-process active attempt"))?;
        let records = self.scan(true)?;
        anyhow::ensure!(
            records.len() == index && record.slot == self.schedule[index],
            "wrong active slot"
        );
        validate_outcome(&record.outcome)?;
        let seal = Seal {
            record_sha256: stutter_core::hash::sha256_hex(&serde_json::to_vec(&record)?),
            record,
        };
        let tmp = format!("{index:04}.tmp");
        let final_name = format!("{index:04}.json");
        self.create_synced(&tmp, &serde_json::to_vec(&seal)?)?;
        // Atomic no-replace rename ensures an existing result is never overwritten.
        crate::syscall::rename_noreplace_at(self.dir.as_fd(), &tmp, &final_name)
            .map_err(|error| anyhow::anyhow!("cannot publish attempt seal: {error}"))?;
        self.dir.sync_all()?;
        self.active = None;
        Ok(())
    }

    fn create_synced(&self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(bytes.len() <= MAX_RECORD as usize, "record too large");
        let mut file = crate::syscall::create_new_at(self.dir.as_fd(), name)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    fn read(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        let mut file = crate::syscall::openat_nofollow(self.dir.as_fd(), name)?;
        let meta = file.metadata()?;
        anyhow::ensure!(
            meta.is_file()
                && meta.nlink() == 1
                && meta.uid() == self.owner_uid
                && meta.mode() & 0o077 == 0
                && meta.len() <= MAX_RECORD,
            "untrusted journal file"
        );
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(MAX_RECORD + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= MAX_RECORD as usize, "journal file too large");
        Ok(bytes)
    }

    fn scan(&self, allow_active: bool) -> anyhow::Result<Vec<AttemptRecord>> {
        // Listing through procfs names the already opened descriptor; all actual reads use openat.
        let path = format!("/proc/self/fd/{}", self.dir.as_raw_fd());
        let mut entries = BTreeSet::new();
        for entry in std::fs::read_dir(path)? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 journal entry"))?;
            anyhow::ensure!(
                entries.insert(name) && entries.len() <= self.schedule.len() * 2,
                "unexpected journal entries"
            );
        }
        let mut records = Vec::new();
        let mut pending = false;
        for (index, slot) in self.schedule.iter().enumerate() {
            let marker = format!("{index:04}.started");
            let result = format!("{index:04}.json");
            let started = entries.remove(&marker);
            let sealed = entries.remove(&result);
            if !started && !sealed {
                pending = true;
                continue;
            }
            anyhow::ensure!(!pending && started, "journal is not a schedule prefix");
            let started: Started = serde_json::from_slice(&self.read(&marker)?)?;
            anyhow::ensure!(
                &started.slot == slot && started.schedule_sha256 == self.schedule_sha256,
                "started slot or frozen schedule mismatch"
            );
            if !sealed {
                anyhow::ensure!(
                    allow_active && self.active == Some(index),
                    "UnsafeToResume: started attempt has no seal"
                );
                pending = true;
                continue;
            }
            let seal: Seal = serde_json::from_slice(&self.read(&result)?)?;
            validate_outcome(&seal.record.outcome)?;
            anyhow::ensure!(
                seal.record.slot == *slot
                    && seal.record_sha256
                        == stutter_core::hash::sha256_hex(&serde_json::to_vec(&seal.record)?),
                "attempt seal mismatch"
            );
            records.push(seal.record);
        }
        anyhow::ensure!(entries.is_empty(), "unknown or temporary journal entry");
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use super::*;

    fn schedule() -> Vec<AttemptSlot> {
        [ProtonSource::Ge, ProtonSource::Em]
            .into_iter()
            .enumerate()
            .map(|(index, arm)| AttemptSlot {
                study_sha256: "a".repeat(64),
                context_sha256: "b".repeat(64),
                index: index as u32,
                arm,
            })
            .collect()
    }

    fn open(path: &std::path::Path) -> anyhow::Result<ResearchJournal> {
        ResearchJournal::open(
            crate::syscall::open_dir_nofollow(path)?,
            crate::syscall::geteuid(),
            schedule(),
        )
    }

    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn record(slot: AttemptSlot) -> AttemptRecord {
        AttemptRecord {
            slot,
            outcome: AttemptOutcome::Completed {
                evidence_sha256: "c".repeat(64),
            },
        }
    }

    #[test]
    fn sealed_prefix_resumes_and_exclusive_lock_rejects_a_duplicated_descriptor() {
        let dir = private_dir();
        let fd = crate::syscall::open_dir_nofollow(dir.path()).unwrap();
        let duplicate = fd.try_clone().unwrap();
        let mut journal = ResearchJournal::open(fd, crate::syscall::geteuid(), schedule()).unwrap();
        assert!(ResearchJournal::open(duplicate, crate::syscall::geteuid(), schedule()).is_err());
        let first = journal.start().unwrap();
        assert!(journal.start().is_err());
        journal.seal(record(first.clone())).unwrap();
        assert_eq!(journal.verify().unwrap(), vec![record(first)]);
        drop(journal);
        let mut resumed = open(dir.path()).unwrap();
        let second = resumed.start().unwrap();
        assert_eq!(second.index, 1);
        resumed.seal(record(second)).unwrap();
        assert_eq!(resumed.verify().unwrap().len(), 2);
        assert!(resumed.start().is_err());
    }

    #[test]
    fn interrupted_attempt_and_temporary_files_are_never_replayed() {
        let dir = private_dir();
        let mut journal = open(dir.path()).unwrap();
        journal.start().unwrap();
        drop(journal);
        assert!(
            matches!(open(dir.path()), Err(error) if error.to_string().contains("UnsafeToResume"))
        );
        let other = private_dir();
        fs::write(other.path().join("0000.tmp"), b"torn").unwrap();
        assert!(open(other.path()).is_err());
    }

    #[test]
    fn wrong_slot_and_bad_result_cannot_create_a_seal() {
        let dir = private_dir();
        let mut journal = open(dir.path()).unwrap();
        let first = journal.start().unwrap();
        let mut wrong_slot = record(first.clone());
        wrong_slot.slot.index = 1;
        assert!(journal.seal(wrong_slot).is_err());
        let mut bad_digest = record(first.clone());
        bad_digest.outcome = AttemptOutcome::Completed {
            evidence_sha256: "0".into(),
        };
        assert!(journal.seal(bad_digest).is_err());
        assert!(!dir.path().join("0000.json").exists());
        assert!(!dir.path().join("0000.tmp").exists());
        journal.seal(record(first)).unwrap();
        fs::copy(
            dir.path().join("0000.started"),
            dir.path().join("0002.started"),
        )
        .unwrap();
        assert!(journal.verify().is_err());
    }

    #[test]
    fn altered_seals_wrong_context_and_symlinks_fail_closed() {
        let dir = private_dir();
        let mut journal = open(dir.path()).unwrap();
        let slot = journal.start().unwrap();
        journal.seal(record(slot)).unwrap();
        drop(journal);
        fs::write(dir.path().join("0000.json"), b"{}").unwrap();
        assert!(open(dir.path()).is_err());

        let dir = private_dir();
        let mut journal = open(dir.path()).unwrap();
        let slot = journal.start().unwrap();
        journal.seal(record(slot)).unwrap();
        drop(journal);
        let mut wrong = schedule();
        for slot in &mut wrong {
            slot.context_sha256 = "d".repeat(64);
        }
        assert!(
            ResearchJournal::open(
                crate::syscall::open_dir_nofollow(dir.path()).unwrap(),
                crate::syscall::geteuid(),
                wrong
            )
            .is_err()
        );
        let mut changed_future_arm = schedule();
        changed_future_arm[1].arm = ProtonSource::ValveExperimental;
        assert!(
            ResearchJournal::open(
                crate::syscall::open_dir_nofollow(dir.path()).unwrap(),
                crate::syscall::geteuid(),
                changed_future_arm,
            )
            .is_err()
        );

        let dir = private_dir();
        symlink("/etc/passwd", dir.path().join("0000.started")).unwrap();
        assert!(open(dir.path()).is_err());
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(open(dir.path()).is_err());
    }
}
