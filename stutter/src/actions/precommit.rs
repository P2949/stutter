//! A prepared transaction that stops before its first mutable write. Nothing was changed, so the
//! candidate is merely stale: callers retire its token and re-observe instead of faulting. The
//! type is the contract; no caller infers it from an error message.
use std::{fmt, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AbortReason {
    /// A target task exited before the first write.
    TargetGone,
    /// The planned placement or value no longer matches the host.
    PlanDrifted,
    /// A task id now names a different task.
    TargetReplaced,
}

#[derive(Debug)]
pub(crate) struct AbortedBeforeMutation {
    pub(crate) reason: AbortReason,
    detail: String,
}

impl AbortedBeforeMutation {
    pub(crate) fn new(reason: AbortReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }

    /// The abort, if any cause in the chain is one.
    pub(crate) fn in_chain(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl fmt::Display for AbortedBeforeMutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for AbortedBeforeMutation {}

/// The second look at a plan, just before the first write. `Ok(())` means the host still matches;
/// otherwise the transaction aborts with zero writes. A failed second look is a vanished target
/// only if one of the planned tasks really is gone; any other failure stays a real error.
pub(crate) fn unchanged<T: PartialEq>(
    second: anyhow::Result<T>,
    first: &T,
    what: &str,
    tids: &[u32],
    proc_root: &Path,
) -> anyhow::Result<()> {
    match second {
        Ok(second) if second == *first => Ok(()),
        Ok(_) => Err(AbortedBeforeMutation::new(
            AbortReason::PlanDrifted,
            format!("{what} drifted between preparation and commit; nothing was written"),
        )
        .into()),
        Err(error) => {
            // Only a missing entry proves the task is gone; an unreadable one is unknown, and
            // unknown stays the original error.
            if let Some(tid) = tids.iter().find(|tid| {
                matches!(
                    std::fs::metadata(proc_root.join(tid.to_string()).join("stat")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound
                )
            }) {
                return Err(AbortedBeforeMutation::new(
                    AbortReason::TargetGone,
                    format!("task {tid} exited before the first write of {what}"),
                )
                .into());
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_vanished_task_turns_a_failed_second_look_into_an_abort() {
        let root = crate::test_support::TestRoot::new("precommit-unchanged");
        let live = root.path().join("7");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join("stat"), "x").unwrap();
        let fail = || -> anyhow::Result<u8> { Err(anyhow::anyhow!("unreadable")) };
        // Task 7 is alive, so an unreadable plan stays a real failure.
        let real = unchanged(fail(), &1, "thing", &[7], root.path()).unwrap_err();
        assert!(AbortedBeforeMutation::in_chain(&real).is_none());
        // Task 8 is gone: clean abort.
        let gone = unchanged(fail(), &1, "thing", &[7, 8], root.path()).unwrap_err();
        assert_eq!(
            AbortedBeforeMutation::in_chain(&gone).unwrap().reason,
            AbortReason::TargetGone
        );
        let drift = unchanged(Ok(2), &1, "thing", &[7], root.path()).unwrap_err();
        assert_eq!(
            AbortedBeforeMutation::in_chain(&drift).unwrap().reason,
            AbortReason::PlanDrifted
        );
        assert!(unchanged(Ok(1), &1, "thing", &[7], root.path()).is_ok());
    }
}
