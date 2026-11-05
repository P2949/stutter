//! Telling a suspend the kernel never entered from one that happened and then misreported.
//!
//! The fixed suspend action hands the machine to `rtcwake`. Only the kernel's own counters say
//! what happened, so the verdict comes from them, with the command's exit status as a second
//! witness. Anything the two witnesses do not agree on is a hard failure, never a retry.
use std::fs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Counters {
    pub(crate) success: u64,
    pub(crate) fail: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    /// The success counter advanced and the command succeeded.
    Verified,
    /// The kernel failed the attempt, never entered sleep, and the command said so too.
    NotEntered,
    /// Anything else: the evidence disagrees or is missing.
    Ambiguous(String),
}

pub(crate) fn classify(before: Counters, after: Counters, command_succeeded: bool) -> Outcome {
    if after.success < before.success || after.fail < before.fail {
        return Outcome::Ambiguous(format!(
            "the kernel suspend counters went backwards ({before:?} then {after:?})"
        ));
    }
    let succeeded = after.success > before.success;
    let failed = after.fail > before.fail;
    match (succeeded, failed, command_succeeded) {
        (true, false, true) => Outcome::Verified,
        (false, true, false) => Outcome::NotEntered,
        (true, _, false) => Outcome::Ambiguous(format!(
            "the machine suspended ({before:?} then {after:?}) but the command reported failure"
        )),
        (true, true, _) => Outcome::Ambiguous(format!(
            "the kernel counted both a success and a failure ({before:?} then {after:?})"
        )),
        (false, false, _) => Outcome::Ambiguous(format!(
            "no suspend counter moved ({before:?} then {after:?}), command succeeded: {command_succeeded}"
        )),
        (false, true, true) => Outcome::Ambiguous(format!(
            "the kernel counted a failed suspend ({before:?} then {after:?}) but the command succeeded"
        )),
    }
}

pub(crate) fn read_counters() -> Result<Counters, String> {
    let read = |name: &str| -> Result<u64, String> {
        fs::read_to_string(format!("/sys/power/suspend_stats/{name}"))
            .map_err(|error| format!("{name}: {error}"))?
            .trim()
            .parse()
            .map_err(|error: std::num::ParseIntError| format!("{name}: {error}"))
    };
    Ok(Counters {
        success: read("success")?,
        fail: read("fail")?,
    })
}

/// The kernel's account of its last failed suspend, for the record only.
pub(crate) fn last_failure() -> String {
    ["last_failed_dev", "last_failed_errno", "last_failed_step"]
        .iter()
        .map(|name| {
            let value = fs::read_to_string(format!("/sys/power/suspend_stats/{name}"))
                .map(|text| text.trim().to_owned())
                .unwrap_or_else(|_| "unreadable".to_owned());
            format!("{name}={value}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: Counters = Counters {
        success: 5,
        fail: 1,
    };

    fn after(success: u64, fail: u64) -> Counters {
        Counters { success, fail }
    }

    #[test]
    fn a_counted_suspend_with_a_successful_command_is_verified() {
        assert_eq!(classify(BEFORE, after(6, 1), true), Outcome::Verified);
    }

    #[test]
    fn a_failed_attempt_the_command_also_reported_was_never_entered() {
        assert_eq!(classify(BEFORE, after(5, 2), false), Outcome::NotEntered);
    }

    #[test]
    fn a_real_suspend_followed_by_a_command_error_is_never_a_retry() {
        assert!(matches!(
            classify(BEFORE, after(6, 1), false),
            Outcome::Ambiguous(_)
        ));
    }

    #[test]
    fn disagreeing_or_missing_evidence_is_never_a_retry() {
        for (counters, command) in [
            (after(5, 1), false),
            (after(5, 1), true),
            (after(5, 2), true),
            (after(6, 2), false),
            (after(6, 2), true),
            (after(4, 1), true),
            (after(5, 0), false),
        ] {
            assert!(
                matches!(classify(BEFORE, counters, command), Outcome::Ambiguous(_)),
                "{counters:?} {command}"
            );
        }
    }
}
