use std::os::unix::net::UnixStream;

use super::*;
use crate::integrations::dbus::{
    DbusClient, Writer,
    fake::{Reply, serve},
};

const SYSTEM76_XML: &str = r#"<node>
  <interface name="com.system76.Scheduler">
    <method name="SetForegroundProcess"><arg type="u" direction="in"/></method>
    <method name="Reload"/>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get"/>
    <property name="Ignored" type="s" access="read"/>
  </interface>
</node>"#;

const READABLE_XML: &str = r#"<node>
  <interface name="x.Manager">
    <method name="Set"/>
    <property name="Mode" type="s" access="read"/>
    <property name="Level" type="u" access="readwrite"/>
    <property name="Secret" type="s" access="write"/>
  </interface>
</node>"#;

#[test]
fn an_interface_with_only_setters_is_unsupported_for_reading_and_names_its_methods() {
    match classify(SYSTEM76_XML, "com.system76.Scheduler") {
        InterfaceSupport::UnsupportedInterface { reason, methods } => {
            assert!(reason.contains("no readable property"));
            assert_eq!(methods, ["SetForegroundProcess", "Reload"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn readable_properties_are_listed_and_write_only_ones_never_count_as_state() {
    let InterfaceSupport::ReadableState { properties } = classify(READABLE_XML, "x.Manager") else {
        panic!("readable");
    };
    assert_eq!(properties.len(), 2);
    assert_eq!(properties["Mode"], "s");
    assert!(!properties.contains_key("Secret"));
    assert!(matches!(
        classify(READABLE_XML, "x.Missing"),
        InterfaceSupport::UnsupportedInterface { .. }
    ));
}

fn bus(owned: bool) -> DbusClient {
    let (ours, theirs) = UnixStream::pair().unwrap();
    serve(theirs, move |call| match call.member.as_str() {
        "GetNameOwner" if owned => {
            let mut body = Writer::default();
            body.string(":1.9");
            Reply::Body("s".into(), body.buf)
        }
        "GetNameOwner" => Reply::Error("org.freedesktop.DBus.Error.NameHasNoOwner"),
        "Introspect" => {
            let mut body = Writer::default();
            body.string(SYSTEM76_XML);
            Reply::Body("s".into(), body.buf)
        }
        _ => Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    });
    DbusClient::from_stream(ours, 1000).unwrap()
}

#[test]
fn the_probe_reports_absent_present_and_never_calls_a_setter() {
    let (service, path, interface) = SYSTEM76;
    assert_eq!(
        probe_interface(&mut bus(false), service, path, interface),
        Exact::Absent
    );
    let Exact::Present(InterfaceSupport::UnsupportedInterface { methods, .. }) =
        probe_interface(&mut bus(true), service, path, interface)
    else {
        panic!("unsupported");
    };
    assert!(methods.contains(&"SetForegroundProcess".to_owned()));
}
