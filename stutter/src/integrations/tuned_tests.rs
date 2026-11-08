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
