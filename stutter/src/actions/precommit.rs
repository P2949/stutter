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
    /// The resource's owner or value is no longer what the receipt proved: someone else holds or
    /// wrote it, or the proof expired. Nothing was written.
    OwnerChanged,
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

/// The ownership check at the mutation boundary: a fresh exact receipt for `key`, read against the
/// value *now*, or the transaction aborts with zero writes. A 30-second cached owner probe is a
/// hint and never stands in for this; each operation family calls it at prepare and again at
/// commit, with a receipt made from the owner's exact interface in between.
pub fn owner_unchanged(
    receipt: &crate::integrations::receipt::ResourceReceipt,
    key: &crate::integrations::receipt::ResourceKey,
    now_ns: u64,
    current_value: &str,
    what: &str,
) -> anyhow::Result<()> {
    receipt
        .permits_mutation(key, now_ns, current_value)
        .map_err(|denial| {
            AbortedBeforeMutation::new(
                AbortReason::OwnerChanged,
                format!(
                    "ownership of {what} is not proven at commit ({denial:?}); nothing was written"
                ),
            )
            .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    mod ownership {
        use super::*;
        use crate::integrations::receipt::{
            LeaveReason, OwnerClass, ResourceKey, ResourceReceipt, RestoreDecision,
            restore_decision,
        };

        const NOW: u64 = 1_000;

        fn key(start_ticks: u64) -> ResourceKey {
            ResourceKey::VmKnob {
                key: format!("vm.swappiness@{start_ticks}"),
                boot_id: "boot".into(),
            }
        }

        fn receipt(owner: OwnerClass, value: &str) -> ResourceReceipt {
            ResourceReceipt {
                key: key(1),
                owner,
                source: "exact interface".into(),
                observed_at_ns: NOW,
                expires_at_ns: NOW + 100,
                observed_value: value.into(),
            }
        }

        fn reason(result: anyhow::Result<()>) -> AbortReason {
            AbortedBeforeMutation::in_chain(&result.unwrap_err())
                .expect("a typed abort")
                .reason
        }

        #[test]
        fn a_fresh_matching_unowned_receipt_permits_the_commit() {
            let held = receipt(OwnerClass::UnmanagedVerified, "60");
            assert!(owner_unchanged(&held, &key(1), NOW + 10, "60", "swappiness").is_ok());
        }

        #[test]
        fn a_writer_after_prepare_an_owner_at_commit_an_expiry_and_a_reused_id_all_abort_before_any_write()
         {
            let held = receipt(OwnerClass::UnmanagedVerified, "60");
            // Someone wrote the knob between prepare and commit.
            assert_eq!(
                reason(owner_unchanged(
                    &held,
                    &key(1),
                    NOW + 10,
                    "10",
                    "swappiness"
                )),
                AbortReason::OwnerChanged
            );
            // A controller took the resource between the probe and the commit: the receipt made
            // at commit names it, so the gate refuses even though the value is unchanged.
            let taken = receipt(OwnerClass::ExternalConfirmed, "60");
            assert_eq!(
                reason(owner_unchanged(
                    &taken,
                    &key(1),
                    NOW + 10,
                    "60",
                    "swappiness"
                )),
                AbortReason::OwnerChanged
            );
            let suspected = receipt(OwnerClass::ExternalSuspected, "60");
            assert!(owner_unchanged(&suspected, &key(1), NOW + 10, "60", "swappiness").is_err());
            // The proof expired (the 30 s cached hint is not a substitute for a fresh receipt).
            assert_eq!(
                reason(owner_unchanged(
                    &held,
                    &key(1),
                    NOW + 500,
                    "60",
                    "swappiness"
                )),
                AbortReason::OwnerChanged
            );
            // The receipt is for a different incarnation of the resource (id reused).
            assert_eq!(
                reason(owner_unchanged(
                    &held,
                    &key(2),
                    NOW + 10,
                    "60",
                    "swappiness"
                )),
                AbortReason::OwnerChanged
            );
        }

        #[test]
        fn rollback_restores_only_what_stutter_applied_and_never_over_a_third_party() {
            // Worker died while applied, the value is still ours: recovery restores.
            assert_eq!(
                restore_decision("10", "10", OwnerClass::StutterOwned),
                RestoreDecision::Restore
            );
            // An external writer changed it afterwards: leave their value alone.
            assert_eq!(
                restore_decision("10", "30", OwnerClass::StutterOwned),
                RestoreDecision::LeaveExternal {
                    reason: LeaveReason::ValueChanged {
                        applied: "10".into(),
                        current: "30".into()
                    }
                }
            );
            // A manager took over (even with the same value): the original is not ours to put back.
            assert_eq!(
                restore_decision("10", "10", OwnerClass::ExternalConfirmed),
                RestoreDecision::LeaveExternal {
                    reason: LeaveReason::OwnerNow(OwnerClass::ExternalConfirmed)
                }
            );
            assert!(matches!(
                restore_decision("10", "10", OwnerClass::Unknown),
                RestoreDecision::LeaveExternal { .. }
            ));
        }
    }

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
