use std::process::Command;

use crate::{
    credentials::{
        CAP_BPF, CAP_PERFMON, CAP_SYS_NICE, CAP_SYS_RESOURCE, CapabilitySet, Identity,
        drop_all_privileges, profiling_capabilities, restrict_child,
    },
    sys,
};

/// Runs `/proc/self/status` through the hardened spawn path and returns the interesting lines.
fn child_status(configure: impl FnOnce(&mut Command)) -> String {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "grep -E '^(CapInh|CapPrm|CapEff|CapAmb|NoNewPrivs|Uid):' /proc/self/status",
    ]);
    configure(&mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn field(status: &str, name: &str) -> String {
    status
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("{name} missing in {status}"))
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn profiling_capability_sets_are_minimal_and_exact() {
    let base = profiling_capabilities(false);
    assert!(
        base.contains(CAP_BPF) && base.contains(CAP_PERFMON) && base.contains(CAP_SYS_RESOURCE)
    );
    assert!(!base.contains(CAP_SYS_NICE));
    let with_nice = profiling_capabilities(true);
    assert!(with_nice.contains(CAP_SYS_NICE) && with_nice.0 & !base.0 == 1_u64 << CAP_SYS_NICE);
    assert!(CapabilitySet::default().is_empty());
}

#[test]
fn an_external_workload_child_has_no_capabilities_and_no_new_privileges() {
    let status = child_status(|command| drop_all_privileges(command, None));
    for name in ["CapInh", "CapPrm", "CapEff", "CapAmb"] {
        assert_eq!(
            field(&status, name),
            "0000000000000000",
            "{name} in {status}"
        );
    }
    assert_eq!(field(&status, "NoNewPrivs"), "1");
}

#[test]
fn the_hardening_holds_even_when_the_parent_carries_capabilities() {
    // This session can hold inheritable capabilities; whatever the parent has, the workload
    // child must end up with none.
    let parent = sys::capabilities().unwrap();
    let status = child_status(|command| drop_all_privileges(command, None));
    assert_eq!(
        field(&status, "CapEff"),
        "0000000000000000",
        "parent held {parent:?}"
    );
    assert_eq!(field(&status, "CapAmb"), "0000000000000000");
}

#[test]
fn a_restricted_child_never_gains_more_than_it_is_given() {
    // Without privilege the unprivileged process cannot hold BPF capabilities, so asking for
    // them must fail closed (the spawn errors) rather than quietly succeed with fewer.
    if sys::effective_uid() == 0 {
        eprintln!("skipped: the fail-closed grant check needs a non-root caller");
        return;
    }
    let mut command = Command::new("/bin/true");
    restrict_child(&mut command, None, profiling_capabilities(false));
    assert!(
        command.spawn().is_err(),
        "an unprivileged caller must not obtain BPF capabilities"
    );
}

#[test]
fn as_root_the_child_runs_as_the_campaign_user_without_capabilities() {
    if sys::effective_uid() != 0 {
        eprintln!(
            "skipped: needs real root to drop to another uid (run via the privileged fixture)"
        );
        return;
    }
    let status = child_status(|command| {
        drop_all_privileges(
            command,
            Some(Identity {
                uid: 65_534,
                gid: 65_534,
            }),
        );
    });
    assert!(field(&status, "Uid") == "65534");
    assert_eq!(field(&status, "CapEff"), "0000000000000000");
    assert_eq!(field(&status, "CapPrm"), "0000000000000000");
}

#[test]
fn capability_masks_round_trip_through_the_syscall_wrappers() {
    let held = sys::capabilities().unwrap();
    // Re-applying exactly what we hold is always permitted and changes nothing.
    sys::set_capabilities(held).unwrap();
    assert_eq!(sys::capabilities().unwrap(), held);
}
