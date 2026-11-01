//! What a physical rollback actually did, per task, so "settled" can be told apart from "left
//! alone". The rollback executors return only a result, so the dispositions travel on the thread
//! that ran them and are collected by whoever audits the rollback.
use std::{cell::Cell, fmt};

/// Counts of how each task of one rollback ended.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RollbackDisposition {
    /// Written back: Stutter still owned the value.
    pub restored: usize,
    /// Preserved: someone else changed the value after Stutter did.
    pub superseded_externally: usize,
    /// Nothing left to restore: the task exited.
    pub target_gone: usize,
    /// Not touched: the task id now belongs to another task.
    pub identity_superseded: usize,
    /// Could not be restored or its ownership could not be established; the token stays open.
    pub unresolved: usize,
}

thread_local! {
    static CURRENT: Cell<RollbackDisposition> = const { Cell::new(RollbackDisposition {
        restored: 0, superseded_externally: 0, target_gone: 0, identity_superseded: 0, unresolved: 0,
    }) };
}

impl RollbackDisposition {
    /// Forgets what an earlier rollback on this thread recorded.
    pub(crate) fn reset() {
        CURRENT.with(|current| current.set(Self::default()));
    }

    pub(crate) fn record(change: impl FnOnce(&mut Self)) {
        CURRENT.with(|current| {
            let mut value = current.get();
            change(&mut value);
            current.set(value);
        });
    }

    /// What this thread recorded since the last `reset`.
    pub(crate) fn take() -> Self {
        CURRENT.with(Cell::take)
    }
}

impl fmt::Display for RollbackDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "restored={} superseded_externally={} target_gone={} identity_superseded={} unresolved={}",
            self.restored,
            self.superseded_externally,
            self.target_gone,
            self.identity_superseded,
            self.unresolved
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions_accumulate_per_thread_until_taken() {
        RollbackDisposition::reset();
        RollbackDisposition::record(|d| d.restored += 1);
        RollbackDisposition::record(|d| d.superseded_externally += 2);
        let seen = RollbackDisposition::take();
        assert_eq!((seen.restored, seen.superseded_externally), (1, 2));
        assert_eq!(RollbackDisposition::take(), RollbackDisposition::default());
    }
}
