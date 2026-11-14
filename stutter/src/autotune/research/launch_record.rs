//! Durable per-attempt launch record, the input recovery needs to tell whether a requested Steam
//! launch is settled. `Prepared` and `Sent` are written before the request can leave; the
//! outcome is written after reconciliation. Every record is private, user-owned and write-once.
//!
//! A request counts as settled only when the manager definitively rejected it, or the unique
//! unit was observed terminal. A missing record, a running unit, an absent unit or an ambiguous
//! outcome is never settled, so an interrupted attempt that may still start stays unsafe.

use std::fs::File;

use serde::{Deserialize, Serialize};

use super::{
    launch_txn::{LaunchOutcome, LaunchPhase, UnitState},
    report_store::{read_private, write_new},
};

const PREPARED: &str = "launch-prepared-v1.json";
const SENT: &str = "launch-sent-v1.json";
const OUTCOME: &str = "launch-outcome-v1.json";

#[derive(Debug, Serialize, Deserialize)]
struct PhaseRecord {
    schema_version: u32,
    boot_id: String,
    unit_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedOutcome {
    Running,
    NeverStarted,
    FinishedAlready,
    Ambiguous,
}

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeRecord {
    schema_version: u32,
    boot_id: String,
    unit_name: String,
    outcome: RecordedOutcome,
}

/// What the attempt directory durably says about this attempt's launch.
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchRecord {
    pub prepared: bool,
    pub sent: bool,
    pub unit_name: Option<String>,
    pub outcome: Option<RecordedOutcome>,
}

fn not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// Persist a launch phase; the `launch_txn::run` persist callback.
pub fn record_phase(dir: &File, phase: LaunchPhase, unit_name: &str) -> anyhow::Result<()> {
    let record = PhaseRecord {
        schema_version: 1,
        boot_id: crate::telemetry::identity::read_boot_id()?,
        unit_name: unit_name.to_owned(),
    };
    let name = match phase {
        LaunchPhase::Prepared => PREPARED,
        LaunchPhase::Sent => SENT,
    };
    write_new(dir, name, &serde_json::to_vec(&record)?)
}

pub fn record_outcome(dir: &File, unit_name: &str, outcome: &LaunchOutcome) -> anyhow::Result<()> {
    let outcome = match outcome {
        LaunchOutcome::Running => RecordedOutcome::Running,
        LaunchOutcome::NeverStarted => RecordedOutcome::NeverStarted,
        LaunchOutcome::FinishedAlready => RecordedOutcome::FinishedAlready,
        LaunchOutcome::Ambiguous(_) => RecordedOutcome::Ambiguous,
    };
    let record = OutcomeRecord {
        schema_version: 1,
        boot_id: crate::telemetry::identity::read_boot_id()?,
        unit_name: unit_name.to_owned(),
        outcome,
    };
    write_new(dir, OUTCOME, &serde_json::to_vec(&record)?)
}

/// Read the durable launch state. An unreadable or inconsistent record is an error, never an
/// absent one; only a clean `NotFound` means "that phase was never written".
pub fn read_launch_record(dir: &File, uid: u32) -> anyhow::Result<LaunchRecord> {
    let phase = |name: &str| -> anyhow::Result<Option<PhaseRecord>> {
        match read_private(dir, name, uid) {
            Ok(bytes) => {
                let record: PhaseRecord = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(record.schema_version == 1, "unsupported launch record");
                Ok(Some(record))
            }
            Err(error) if not_found(&error) => Ok(None),
            Err(error) => Err(error),
        }
    };
    let prepared = phase(PREPARED)?;
    let sent = phase(SENT)?;
    let outcome = match read_private(dir, OUTCOME, uid) {
        Ok(bytes) => {
            let record: OutcomeRecord = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(record.schema_version == 1, "unsupported launch outcome");
            Some(record)
        }
        Err(error) if not_found(&error) => None,
        Err(error) => return Err(error),
    };
    // The phases are written in order: a later record without its predecessor is corruption.
    anyhow::ensure!(
        (sent.is_none() || prepared.is_some()) && (outcome.is_none() || sent.is_some()),
        "launch records are out of order"
    );
    // Every record must come from this boot: a record from another boot cannot be compared with
    // present unit, process or worker observations.
    let boot = crate::telemetry::identity::read_boot_id()?;
    anyhow::ensure!(
        [
            prepared.as_ref().map(|r| r.boot_id.as_str()),
            sent.as_ref().map(|r| r.boot_id.as_str()),
            outcome.as_ref().map(|r| r.boot_id.as_str()),
        ]
        .into_iter()
        .flatten()
        .all(|recorded| recorded == boot),
        "launch records are from a different boot"
    );
    let unit_names: Vec<&str> = [
        prepared.as_ref().map(|r| r.unit_name.as_str()),
        sent.as_ref().map(|r| r.unit_name.as_str()),
        outcome.as_ref().map(|r| r.unit_name.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect();
    anyhow::ensure!(
        unit_names.windows(2).all(|pair| pair[0] == pair[1]),
        "launch records disagree on the unit"
    );
    Ok(LaunchRecord {
        prepared: prepared.is_some(),
        sent: sent.is_some(),
        unit_name: unit_names.first().map(|name| (*name).to_owned()),
        outcome: outcome.map(|record| record.outcome),
    })
}

/// Whether the requested launch can no longer start anything. `current` is a fresh observation of
/// the unique unit (or `None` when it could not be taken). Never true on ambiguity.
pub fn request_settled(record: &LaunchRecord, current: Option<UnitState>) -> bool {
    if !record.sent {
        // Prepared but never sent leaves nothing that could execute.
        return true;
    }
    matches!(
        (record.outcome, current),
        (Some(RecordedOutcome::NeverStarted), _) | (_, Some(UnitState::Finished))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (tempfile::TempDir, File) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            tmp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let file = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        (tmp, file)
    }

    fn uid() -> u32 {
        crate::syscall::geteuid()
    }

    const UNIT: &str = "stutter-research-0123456789abcdef-0000.service";

    #[test]
    fn an_empty_directory_has_no_launch_and_is_settled() {
        let (_tmp, dir) = fresh();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert_eq!(
            record,
            LaunchRecord {
                prepared: false,
                sent: false,
                unit_name: None,
                outcome: None
            }
        );
        assert!(request_settled(&record, None));
    }

    #[test]
    fn a_sent_request_without_a_terminal_observation_is_never_settled() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        // Prepared only: nothing could have been sent.
        assert!(request_settled(
            &read_launch_record(&dir, uid()).unwrap(),
            None
        ));
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert!(record.sent && record.outcome.is_none());
        for current in [None, Some(UnitState::Absent), Some(UnitState::Active)] {
            assert!(!request_settled(&record, current));
        }
        assert!(request_settled(&record, Some(UnitState::Finished)));
    }

    #[test]
    fn only_a_rejection_or_a_terminal_unit_settles_and_ambiguity_never_does() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome(&dir, UNIT, &LaunchOutcome::Ambiguous("lost")).unwrap();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert_eq!(record.outcome, Some(RecordedOutcome::Ambiguous));
        assert!(!request_settled(&record, Some(UnitState::Absent)));
        let settled = |outcome: RecordedOutcome| LaunchRecord {
            prepared: true,
            sent: true,
            unit_name: Some(UNIT.into()),
            outcome: Some(outcome),
        };
        assert!(request_settled(
            &settled(RecordedOutcome::NeverStarted),
            None
        ));
        assert!(!request_settled(&settled(RecordedOutcome::Running), None));
        assert!(request_settled(
            &settled(RecordedOutcome::Running),
            Some(UnitState::Finished)
        ));
    }

    #[test]
    fn records_from_another_boot_are_refused() {
        let (_tmp, dir) = fresh();
        let forged = format!(
            r#"{{"schema_version":1,"boot_id":"{}","unit_name":"{UNIT}"}}"#,
            "0".repeat(32)
        );
        write_new(&dir, PREPARED, forged.as_bytes()).unwrap();
        assert!(read_launch_record(&dir, uid()).is_err());
    }

    #[test]
    fn records_are_write_once_ordered_and_consistent() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        // A second write of the same phase is refused, never overwritten.
        assert!(record_phase(&dir, LaunchPhase::Prepared, UNIT).is_err());
        // A later record without its predecessor is corruption.
        let (_tmp2, dir2) = fresh();
        record_phase(&dir2, LaunchPhase::Sent, UNIT).unwrap();
        assert!(read_launch_record(&dir2, uid()).is_err());
        // Records that name different units are refused.
        let (_tmp3, dir3) = fresh();
        record_phase(&dir3, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir3, LaunchPhase::Sent, "other.service").unwrap();
        assert!(read_launch_record(&dir3, uid()).is_err());
    }
}
