use crate::actions::{PartialApplyError, RollbackToken};

pub struct ApplyTransaction<R> {
    pub planned: Vec<R>,
    pub applied: Vec<R>,
}

impl<R> ApplyTransaction<R> {
    pub fn new() -> Self {
        Self {
            planned: Vec::new(),
            applied: Vec::new(),
        }
    }

    pub fn plan(&mut self, record: R) {
        self.planned.push(record);
    }

    pub fn mark_applied(&mut self, record: R) {
        self.applied.push(record);
    }

    pub fn partial_token<T>(self, make_token: T) -> Option<RollbackToken>
    where
        T: FnOnce(Vec<R>) -> RollbackToken,
    {
        (!self.applied.is_empty()).then(|| make_token(self.applied))
    }

    #[cfg(test)]
    pub fn rollback_applied<F>(self, mut rollback_fn: F) -> anyhow::Result<()>
    where
        F: FnMut(&R) -> anyhow::Result<()>,
    {
        for record in self.applied.iter().rev() {
            rollback_fn(record)?;
        }

        Ok(())
    }

    pub fn apply_planned_loop<I, F, T>(
        mut self,
        items: I,
        mut apply_fn: F,
        make_token: T,
    ) -> Result<RollbackToken, PartialApplyError>
    where
        I: IntoIterator<Item = R>,
        F: FnMut(&R) -> Result<(), anyhow::Error>,
        T: FnOnce(Vec<R>) -> RollbackToken,
    {
        for item in items {
            self.plan(item);
        }

        for record in std::mem::take(&mut self.planned) {
            match apply_fn(&record) {
                Ok(()) => {
                    self.mark_applied(record);
                }
                Err(err) => {
                    let rollback = self.partial_token(make_token);
                    return Err(PartialApplyError {
                        source: err,
                        rollback,
                    });
                }
            }
        }
        Ok(make_token(self.applied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{
        IoPrioRestoreRecord, NiceRestoreRecord, TaskRestoreIdentity, UclampRestoreRecord,
    };

    fn identity(tid: u32, comm: &str) -> TaskRestoreIdentity {
        TaskRestoreIdentity::observed(
            tid,
            Some(tid),
            Some(comm.to_owned()),
            Some(u64::from(tid) + 1_000),
            None,
        )
    }

    #[test]
    fn planned_loop_returns_partial_token_for_applied_prefix() {
        let err = ApplyTransaction::new()
            .apply_planned_loop(
                vec!["first", "second", "third"],
                |record| {
                    if *record == "second" {
                        anyhow::bail!("second failed");
                    }

                    Ok(())
                },
                |records| RollbackToken::VmKnobRestore {
                    records: records
                        .into_iter()
                        .map(|record| crate::actions::VmKnobRestoreRecord {
                            path: std::path::PathBuf::from(record),
                            original_value: "old".to_owned(),
                        })
                        .collect(),
                },
            )
            .unwrap_err();

        assert!(format!("{:#}", err.source).contains("second failed"));
        let rollback = err.rollback.expect("partial rollback token");
        let RollbackToken::VmKnobRestore { records } = rollback else {
            panic!("unexpected rollback token");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, std::path::PathBuf::from("first"));
    }

    #[test]
    fn rollback_applied_visits_records_in_reverse_order() {
        let mut tx = ApplyTransaction::new();
        tx.mark_applied(1);
        tx.mark_applied(2);

        let mut rolled_back = Vec::new();
        tx.rollback_applied(|record| {
            rolled_back.push(*record);
            Ok(())
        })
        .unwrap();

        assert_eq!(rolled_back, vec![2, 1]);
    }

    #[test]
    fn nice_partial_token_contains_applied_restore_records() {
        let records = vec![
            NiceRestoreRecord::new(identity(42, "game-thread"), 0),
            NiceRestoreRecord::new(identity(43, "game-worker"), 5),
        ];

        let err = ApplyTransaction::new()
            .apply_planned_loop(
                records,
                |record| {
                    if record.tid() == 43 {
                        anyhow::bail!("simulated nice failure");
                    }
                    Ok(())
                },
                |records| RollbackToken::NiceRestore { records },
            )
            .unwrap_err();

        let RollbackToken::NiceRestore { records } =
            err.rollback.expect("partial nice rollback token")
        else {
            panic!("unexpected rollback token");
        };

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tid(), 42);
        assert_eq!(records[0].original_nice, 0);
    }

    #[test]
    fn ioprio_partial_token_contains_applied_restore_records() {
        let records = vec![
            IoPrioRestoreRecord::new(identity(42, "storage-worker"), 4),
            IoPrioRestoreRecord::new(identity(43, "storage-helper"), 7),
        ];

        let err = ApplyTransaction::new()
            .apply_planned_loop(
                records,
                |record| {
                    if record.tid() == 43 {
                        anyhow::bail!("simulated ioprio failure");
                    }
                    Ok(())
                },
                |records| RollbackToken::IoPrioRestore { records },
            )
            .unwrap_err();

        let RollbackToken::IoPrioRestore { records } =
            err.rollback.expect("partial ioprio rollback token")
        else {
            panic!("unexpected rollback token");
        };

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tid(), 42);
        assert_eq!(records[0].original_ioprio, 4);
    }

    #[test]
    fn uclamp_partial_token_contains_applied_restore_records() {
        let records = vec![
            UclampRestoreRecord::new(identity(42, "game-thread"), 0, 1024),
            UclampRestoreRecord::new(identity(43, "game-worker"), 128, 900),
        ];

        let err = ApplyTransaction::new()
            .apply_planned_loop(
                records,
                |record| {
                    if record.tid() == 43 {
                        anyhow::bail!("simulated uclamp failure");
                    }
                    Ok(())
                },
                |records| RollbackToken::UclampRestore { records },
            )
            .unwrap_err();

        let RollbackToken::UclampRestore { records } =
            err.rollback.expect("partial uclamp rollback token")
        else {
            panic!("unexpected rollback token");
        };

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tid(), 42);
        assert_eq!(records[0].original_util_min, 0);
        assert_eq!(records[0].original_util_max, 1024);
    }
}
