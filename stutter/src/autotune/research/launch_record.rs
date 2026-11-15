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
    /// The unit's systemd invocation id when the launch was confirmed, if it was readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invocation_id: Option<String>,
}

/// What the attempt directory durably says about this attempt's launch.
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchRecord {
    pub prepared: bool,
    pub sent: bool,
    pub unit_name: Option<String>,
    pub outcome: Option<RecordedOutcome>,
    pub invocation_id: Option<String>,
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
    record_outcome_with_invocation(dir, unit_name, outcome, None)
}

/// Record the outcome together with the confirmed unit's systemd invocation id, so a later
/// observation can be tied to the request that was actually sent.
pub fn record_outcome_with_invocation(
    dir: &File,
    unit_name: &str,
    outcome: &LaunchOutcome,
    invocation_id: Option<&str>,
) -> anyhow::Result<()> {
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
        invocation_id: invocation_id.map(str::to_owned),
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
        invocation_id: outcome
            .as_ref()
            .and_then(|record| record.invocation_id.clone()),
        outcome: outcome.map(|record| record.outcome),
    })
}

/// Whether the requested launch can no longer start anything. `current` is a fresh observation of
/// the unique unit (or `None` when it could not be taken). Never true on ambiguity.
pub fn request_settled(
    record: &LaunchRecord,
    current: Option<UnitState>,
    current_invocation: Option<&str>,
) -> bool {
    if !record.sent {
        // Prepared but never sent leaves nothing that could execute.
        return true;
    }
    // The recorded outcome and the fresh observation must agree. A unit that was never started
    // but is seen, or a launch we recorded as ambiguous (or never reconciled) that is later seen
    // finished, is contradictory or unattributed evidence and never settles.
    match (record.outcome, current) {
        (Some(RecordedOutcome::NeverStarted), None | Some(UnitState::Absent)) => true,
        (Some(RecordedOutcome::Running | RecordedOutcome::FinishedAlready), observed) => {
            match (&record.invocation_id, observed) {
                // A transient launcher unit that exits cleanly is collected, so a unit that is gone
                // after we confirmed this very invocation running means that invocation ended: a
                // transient unit cannot be restarted under the same invocation, and any foreign
                // replacement would exist and be active, not absent.
                (Some(_), Some(UnitState::Absent)) => true,
                // A unit that still exists settles only if it is the same invocation we confirmed.
                (Some(recorded), Some(UnitState::Finished)) => {
                    current_invocation == Some(recorded.as_str())
                }
                // Without a recorded invocation, nothing binds a terminal or absent unit to this
                // request: a same-named unit may be a replacement. The launch stays unresolved.
                _ => false,
            }
        }
        _ => false,
    }
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
                outcome: None,
                invocation_id: None,
            }
        );
        assert!(request_settled(&record, None, None));
    }

    #[test]
    fn a_sent_request_without_a_terminal_observation_is_never_settled() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        // Prepared only: nothing could have been sent.
        assert!(request_settled(
            &read_launch_record(&dir, uid()).unwrap(),
            None,
            None
        ));
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert!(record.sent && record.outcome.is_none());
        for current in [None, Some(UnitState::Absent), Some(UnitState::Active)] {
            assert!(!request_settled(&record, current, None));
        }
        // A finished unit with no reconciled outcome is unattributed: never settled.
        assert!(!request_settled(&record, Some(UnitState::Finished), None));
    }

    #[test]
    fn only_a_rejection_or_a_terminal_unit_settles_and_ambiguity_never_does() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome(&dir, UNIT, &LaunchOutcome::Ambiguous("lost")).unwrap();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert_eq!(record.outcome, Some(RecordedOutcome::Ambiguous));
        assert!(!request_settled(&record, Some(UnitState::Absent), None));
        let settled = |outcome: RecordedOutcome| LaunchRecord {
            prepared: true,
            sent: true,
            unit_name: Some(UNIT.into()),
            outcome: Some(outcome),
            invocation_id: None,
        };
        assert!(request_settled(
            &settled(RecordedOutcome::NeverStarted),
            None,
            None
        ));
        assert!(!request_settled(
            &settled(RecordedOutcome::Running),
            None,
            None
        ));
        // Contradictions never settle: never started yet a unit is seen, or ambiguous yet finished.
        for seen in [UnitState::Active, UnitState::Finished] {
            assert!(!request_settled(
                &settled(RecordedOutcome::NeverStarted),
                Some(seen),
                None
            ));
        }
        assert!(!request_settled(
            &settled(RecordedOutcome::Ambiguous),
            Some(UnitState::Finished),
            None
        ));
        assert!(!request_settled(
            &settled(RecordedOutcome::Running),
            Some(UnitState::Absent),
            None
        ));
        // A confirmed launch whose invocation was never recorded is unresolved: a finished unit with
        // the same name may be a replacement, so nothing binds it to this request.
        for outcome in [RecordedOutcome::Running, RecordedOutcome::FinishedAlready] {
            for observed in [UnitState::Finished, UnitState::Absent, UnitState::Active] {
                assert!(!request_settled(&settled(outcome), Some(observed), None));
                assert!(!request_settled(
                    &settled(outcome),
                    Some(observed),
                    Some("any")
                ));
            }
            assert!(!request_settled(&settled(outcome), None, None));
        }
    }

    #[test]
    fn a_replaced_same_named_unit_never_settles_the_original_request() {
        let record = LaunchRecord {
            prepared: true,
            sent: true,
            unit_name: Some(UNIT.into()),
            outcome: Some(RecordedOutcome::Running),
            invocation_id: Some("a".repeat(32)),
        };
        let other = "b".repeat(32);
        assert!(!request_settled(
            &record,
            Some(UnitState::Finished),
            Some(other.as_str())
        ));
        assert!(!request_settled(&record, Some(UnitState::Finished), None));
        assert!(!request_settled(
            &record,
            Some(UnitState::Active),
            Some(other.as_str())
        ));
        assert!(request_settled(
            &record,
            Some(UnitState::Finished),
            Some("a".repeat(32).as_str())
        ));
        assert!(request_settled(&record, Some(UnitState::Absent), None));
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

    fn confirmed(outcome: RecordedOutcome, invocation: Option<&str>) -> LaunchRecord {
        LaunchRecord {
            prepared: true,
            sent: true,
            unit_name: Some(UNIT.into()),
            outcome: Some(outcome),
            invocation_id: invocation.map(str::to_owned),
        }
    }

    #[test]
    fn a_confirmed_invocation_settles_when_it_is_gone_or_finished_and_never_when_replaced() {
        let record = confirmed(RecordedOutcome::Running, Some("aa"));
        // A cleanly exited transient launcher is collected: that exact invocation ended.
        assert!(request_settled(&record, Some(UnitState::Absent), None));
        // A unit that still exists settles only as the same terminal invocation.
        assert!(request_settled(
            &record,
            Some(UnitState::Finished),
            Some("aa")
        ));
        assert!(!request_settled(
            &record,
            Some(UnitState::Finished),
            Some("bb")
        ));
        assert!(!request_settled(&record, Some(UnitState::Finished), None));
        // Running or starting is never settled, and an unobserved unit is unknown.
        assert!(!request_settled(
            &record,
            Some(UnitState::Active),
            Some("aa")
        ));
        assert!(!request_settled(&record, None, None));
    }

    #[test]
    fn without_a_confirmed_invocation_absence_is_never_settled() {
        let record = confirmed(RecordedOutcome::Running, None);
        assert!(!request_settled(&record, Some(UnitState::Absent), None));
        assert!(!request_settled(&record, Some(UnitState::Finished), None));
        assert!(!request_settled(
            &record,
            Some(UnitState::Finished),
            Some("aa")
        ));
        // An ambiguous or never-started outcome is unaffected by any invocation.
        let ambiguous = confirmed(RecordedOutcome::Ambiguous, Some("aa"));
        assert!(!request_settled(&ambiguous, Some(UnitState::Absent), None));
        assert!(!request_settled(
            &ambiguous,
            Some(UnitState::Finished),
            Some("aa")
        ));
    }

    #[test]
    fn the_invocation_id_round_trips_through_the_durable_outcome() {
        use super::super::launch_txn::LaunchOutcome;
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome_with_invocation(&dir, UNIT, &LaunchOutcome::Running, Some("0123abcd"))
            .unwrap();
        let record = read_launch_record(&dir, uid()).unwrap();
        assert_eq!(record.invocation_id.as_deref(), Some("0123abcd"));
        // A record written without one reads back as unknown, never as a wildcard.
        let (_tmp2, dir2) = fresh();
        record_phase(&dir2, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir2, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome(&dir2, UNIT, &LaunchOutcome::Running).unwrap();
        assert_eq!(
            read_launch_record(&dir2, uid()).unwrap().invocation_id,
            None
        );
    }
}
