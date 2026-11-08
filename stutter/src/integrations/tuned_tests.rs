use std::os::unix::net::UnixStream;

use super::*;
use crate::integrations::dbus::{Writer, fake};

fn bus(answer: impl Fn(&fake::Call) -> fake::Reply + Send + 'static) -> (DbusClient, fake::Calls) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let calls = fake::serve(theirs, answer);
    (DbusClient::from_stream(ours, 1000).unwrap(), calls)
}

fn string_reply(text: &str) -> fake::Reply {
    let mut body = Writer::default();
    body.string(text);
    fake::Reply::Body("s".to_owned(), body.buf)
}

#[test]
fn the_active_profile_and_mode_are_read_with_read_only_calls_only() {
    let (mut client, calls) = bus(
        |call| match (call.member.as_str(), call.destination.as_str()) {
            ("GetNameOwner", _) => string_reply(":1.7"),
            ("active_profile", "com.redhat.tuned") => string_reply("throughput-performance"),
            ("profile_mode", "com.redhat.tuned") => string_reply("auto"),
            _ => fake::Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
        },
    );
    assert_eq!(
        read_tuned(&mut client),
        Exact::Present(TunedState {
            active_profile: "throughput-performance".to_owned(),
            mode: "auto".to_owned()
        })
    );
    let calls = calls.lock().unwrap();
    assert!(calls.iter().all(|call| matches!(
        call.member.as_str(),
        "GetNameOwner" | "active_profile" | "profile_mode"
    )));
}

#[test]
fn an_unowned_name_is_absent_and_an_error_is_unreachable_never_absent() {
    let (mut client, _) = bus(|_| fake::Reply::Error("org.freedesktop.DBus.Error.NameHasNoOwner"));
    assert_eq!(read_tuned(&mut client), Exact::Absent);

    let (mut client, _) = bus(|call| match call.member.as_str() {
        "GetNameOwner" => string_reply(":1.7"),
        _ => fake::Reply::Error("org.freedesktop.DBus.Error.AccessDenied"),
    });
    assert!(matches!(read_tuned(&mut client), Exact::Unreachable(_)));
}

#[test]
fn an_unreadable_mode_degrades_to_unknown_not_to_a_guess() {
    let (mut client, _) = bus(|call| match call.member.as_str() {
        "GetNameOwner" => string_reply(":1.7"),
        "active_profile" => string_reply("balanced"),
        _ => fake::Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    });
    let Exact::Present(state) = read_tuned(&mut client) else {
        panic!("profile is readable");
    };
    assert_eq!(state.mode, "unknown");
}

fn profiles(files: &[(&str, &str)], tag: &str) -> Vec<std::path::PathBuf> {
    let root = std::env::temp_dir().join(format!("stutter-tuned-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (name, text) in files {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tuned.conf"), text).unwrap();
    }
    vec![root]
}

#[test]
fn a_profile_and_its_includes_yield_the_sections_sysctl_keys_and_domains_it_manages() {
    let roots = profiles(
        &[
            (
                "gaming",
                "[main]\ninclude=base, extra\n[cpu]\ngovernor=performance\n",
            ),
            (
                "base",
                "[main]\nsummary=x\n[sysctl]\nvm.swappiness=10\nnet.core.somaxconn=1024\n",
            ),
            ("extra", "# comment\n[disk]\nreadahead=4096\n"),
        ],
        "a",
    );
    let attributes = read_managed_attributes(&roots, "gaming").unwrap();
    assert_eq!(attributes.profiles.len(), 3);
    assert!(attributes.sections.contains("cpu") && attributes.sections.contains("disk"));
    assert!(attributes.sysctl_keys.contains("vm.swappiness"));
    let domains = attributes.domains();
    assert!(
        domains.contains(&ResourceDomain::CpuPower) && domains.contains(&ResourceDomain::VmKnobs)
    );
}

#[test]
fn an_unreadable_profile_a_path_trick_and_a_cycle_are_handled_without_guessing() {
    let roots = profiles(
        &[
            ("loop-a", "[main]\ninclude=loop-b\n"),
            ("loop-b", "[main]\ninclude=loop-a\n"),
            ("net-only", "[net]\nchannels=combined 4\n"),
        ],
        "b",
    );
    assert_eq!(read_managed_attributes(&roots, "missing"), None);
    assert_eq!(read_managed_attributes(&roots, "../etc"), None);
    assert_eq!(read_managed_attributes(&roots, ""), None);
    assert_eq!(
        read_managed_attributes(&roots, "loop-a")
            .unwrap()
            .profiles
            .len(),
        2
    );
    assert!(
        read_managed_attributes(&roots, "net-only")
            .unwrap()
            .domains()
            .is_empty()
    );
}

#[test]
fn only_an_exactly_read_profile_that_manages_nothing_stunes_lowers_the_claim() {
    use crate::integrations::{Daemon, KnownOwner, OwnerReport, SchedExt};
    let running = || OwnerReport {
        sched_ext: SchedExt::Disabled,
        daemons: crate::integrations::owner_domains()
            .into_iter()
            .map(|(owner, _)| {
                (
                    owner,
                    if owner == KnownOwner::Tuned {
                        Daemon::Running
                    } else {
                        Daemon::NotRunning
                    },
                )
            })
            .collect(),
    };
    let none = ManagedAttributes::default();
    let mut idle = running();
    refine_tuned(&mut idle, Some(&none));
    assert_eq!(idle.daemon(KnownOwner::Tuned), Daemon::Idle);

    let mut managing = ManagedAttributes::default();
    managing.sections.insert("cpu".into());
    for case in [Some(&managing), None] {
        let mut report = running();
        refine_tuned(&mut report, case);
        assert_eq!(report.daemon(KnownOwner::Tuned), Daemon::Running);
    }
}

#[test]
fn a_profile_experiment_artifact_names_exactly_what_the_two_profiles_manage_differently() {
    let roots = profiles(
        &[
            (
                "balanced",
                "[cpu]\ngovernor=schedutil\n[sysctl]\nvm.swappiness=60\n",
            ),
            (
                "throughput",
                "[cpu]\ngovernor=performance\n[sysctl]\nvm.swappiness=10\nkernel.sched_x=1\n[disk]\nreadahead=1\n",
            ),
        ],
        "c",
    );
    let artifact = profile_experiment(&roots, "balanced", "throughput").unwrap();
    assert_eq!(artifact.restore_to, "balanced");
    assert!(artifact.sections_only_in_candidate.contains("disk"));
    assert!(artifact.sections_only_in_baseline.is_empty());
    assert!(artifact.sysctl_only_in_candidate.contains("kernel.sched_x"));
    assert!(
        artifact.differing_domains.is_empty(),
        "both manage CPU power and VM knobs"
    );
    assert_eq!(profile_experiment(&roots, "balanced", "balanced"), None);
    assert_eq!(profile_experiment(&roots, "balanced", "missing"), None);
}
