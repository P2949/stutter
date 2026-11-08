use std::os::unix::net::UnixStream;

use super::*;
use crate::integrations::{
    ResourceDomain, SchedExt,
    dbus::{DbusClient, Writer, fake},
};

fn client(
    answer: impl Fn(&fake::Call) -> fake::Reply + Send + 'static,
) -> (DbusClient, fake::Calls) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let calls = fake::serve(theirs, answer);
    (DbusClient::from_stream(ours, 1000).unwrap(), calls)
}

fn path_reply(path: &str) -> fake::Reply {
    let mut body = Writer::default();
    body.string(path);
    fake::Reply::Body("o".to_owned(), body.buf)
}

fn variant_string(text: &str) -> fake::Reply {
    let mut body = Writer::default();
    body.signature("s");
    body.string(text);
    fake::Reply::Body("v".to_owned(), body.buf)
}

fn systemd(active: &'static str) -> impl Fn(&fake::Call) -> fake::Reply + Send + 'static {
    move |call| match call.member.as_str() {
        "GetUnit" => {
            if call.body.windows(11).any(|w| w == b"tlp.service") {
                path_reply("/org/freedesktop/systemd1/unit/tlp_2eservice")
            } else {
                fake::Reply::Error("org.freedesktop.systemd1.NoSuchUnit")
            }
        }
        "Get" => variant_string(active),
        _ => fake::Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    }
}

fn idle_report() -> OwnerReport {
    OwnerReport {
        sched_ext: SchedExt::Disabled,
        daemons: crate::integrations::owner_domains()
            .into_iter()
            .map(|(owner, _)| (owner, Daemon::NotRunning))
            .collect(),
    }
}

#[test]
fn an_active_unit_reads_back_and_an_unloaded_unit_is_absent() {
    let (mut client, calls) = client(systemd("active"));
    assert_eq!(
        unit_active_state(&mut client, "tlp.service"),
        Exact::Present("active".to_owned())
    );
    assert_eq!(
        unit_active_state(&mut client, "thermald.service"),
        Exact::Absent
    );
    let calls = calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .all(|call| matches!(call.member.as_str(), "GetUnit" | "Get"))
    );
    assert!(calls.iter().all(|call| call.destination == SYSTEMD));
}

#[test]
fn a_bad_unit_name_is_never_sent() {
    let (mut client, calls) = client(systemd("active"));
    for bad in ["", "../x", "a b", "tlp.service\n"] {
        assert!(
            matches!(unit_active_state(&mut client, bad), Exact::Unreachable(_)),
            "{bad:?}"
        );
    }
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn only_an_exactly_active_unit_raises_a_claim() {
    for (state, running) in [
        ("active", true),
        ("activating", true),
        ("inactive", false),
        ("failed", false),
    ] {
        let (mut client, _) = client(systemd(state));
        let states = read_unit_states(&mut client);
        let mut report = idle_report();
        refine_with_units(&mut report, &states);
        assert_eq!(
            report.daemon(KnownOwner::Tlp) == Daemon::Running,
            running,
            "{state}"
        );
        assert_eq!(report.daemon(KnownOwner::Thermald), Daemon::NotRunning);
    }
}

#[test]
fn an_active_tlp_unit_owns_cpu_power_though_no_process_is_visible() {
    let (mut client, _) = client(systemd("active"));
    let mut report = idle_report();
    assert_eq!(report.external_owner(ResourceDomain::CpuPower), None);
    refine_with_units(&mut report, &read_unit_states(&mut client));
    assert!(report.external_owner(ResourceDomain::CpuPower).is_some());
}

#[test]
fn unreachable_units_never_lower_or_raise_anything() {
    let mut report = idle_report();
    report.daemons.insert(KnownOwner::Tuned, Daemon::Running);
    let before = report.clone();
    let states = OWNER_UNITS
        .iter()
        .map(|(owner, _)| (*owner, Exact::Unreachable("bus down".to_owned())))
        .collect::<Vec<_>>();
    refine_with_units(&mut report, &states);
    assert_eq!(report, before);
}
