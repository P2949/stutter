use std::{
    collections::BTreeMap,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
};

use super::*;
use crate::integrations::dbus::{
    DbusClient, Writer,
    fake::{Call, Reply, serve},
    fake_reader,
};

const UNIT: &str = "stutter-helper.service";

type State = Arc<Mutex<BTreeMap<String, u64>>>;

/// A fake systemd: `GetUnit` names a path, `Get` reads `state`, and `SetUnitProperties` writes it
/// unless `ignore_sets` (a unit that refuses the change).
fn systemd(
    initial: &[(&str, u64)],
    ignore_sets: bool,
) -> (DbusClient, Arc<Mutex<Vec<Call>>>, State) {
    let state: State = Arc::new(Mutex::new(
        initial.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    ));
    let (ours, theirs) = UnixStream::pair().unwrap();
    let shared = Arc::clone(&state);
    let calls = serve(theirs, move |call| match call.member.as_str() {
        "GetUnit" => {
            let mut body = Writer::default();
            body.string("/org/freedesktop/systemd1/unit/stutter_2dhelper_2eservice");
            Reply::Body("o".to_owned(), body.buf)
        }
        "Get" => {
            let mut reader = fake_reader(&call.body);
            let _interface = reader.string().unwrap();
            let property = reader.string().unwrap();
            let value = shared.lock().unwrap().get(&property).copied().unwrap_or(0);
            let mut body = Writer::default();
            body.signature("t");
            body.u64(value);
            Reply::Body("v".to_owned(), body.buf)
        }
        "SetUnitProperties" => {
            let mut reader = fake_reader(&call.body);
            let _unit = reader.string().unwrap();
            let runtime = reader.u32().unwrap();
            assert_eq!(runtime, 1, "a unit-file write was requested");
            let Value::Array(items) = reader.value("a(sv)").unwrap() else {
                panic!("not an array");
            };
            if !ignore_sets {
                for item in items {
                    let Value::Struct(parts) = item else { panic!() };
                    let (Value::Str(name), Value::Variant(inner)) = (&parts[0], &parts[1]) else {
                        panic!("{parts:?}")
                    };
                    let Value::U64(value) = **inner else { panic!() };
                    shared.lock().unwrap().insert(name.clone(), value);
                }
            }
            Reply::Body(String::new(), Vec::new())
        }
        _ => Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    });
    (DbusClient::from_stream(ours, 0).unwrap(), calls, state)
}

fn current() -> Vec<(UnitProperty, u64)> {
    vec![
        (UnitProperty::CpuWeight, 100),
        (UnitProperty::MemoryMax, UNSET),
    ]
}

#[test]
fn a_plan_exists_only_for_an_owned_unit_with_bounded_distinct_values_and_known_originals() {
    let change = [(UnitProperty::CpuWeight, 200)];
    let plan = plan_properties(UNIT, &change, &current()).unwrap();
    assert_eq!(plan.restore, [(UnitProperty::CpuWeight, 100)]);
    for unit in [
        "sshd.service",
        "stutter-x.timer",
        "stutter-.service",
        "stutter-a/b.service",
        "stutter-x",
        "user.slice",
    ] {
        let error = plan_properties(unit, &change, &current()).unwrap_err();
        assert_eq!(error, PlanDenial::NotOwnedUnit, "{unit}");
    }
    assert_eq!(
        plan_properties(UNIT, &[], &current()),
        Err(PlanDenial::EmptyChange)
    );
    assert_eq!(
        plan_properties(UNIT, &[(UnitProperty::CpuWeight, 0)], &current()),
        Err(PlanDenial::BadValue(UnitProperty::CpuWeight))
    );
    assert_eq!(
        plan_properties(UNIT, &[(UnitProperty::IoWeight, 10_001)], &current()),
        Err(PlanDenial::BadValue(UnitProperty::IoWeight))
    );
    assert_eq!(
        plan_properties(UNIT, &[(UnitProperty::MemoryMax, 0)], &current()),
        Err(PlanDenial::BadValue(UnitProperty::MemoryMax))
    );
    assert_eq!(
        plan_properties(
            UNIT,
            &[(UnitProperty::CpuWeight, 5), (UnitProperty::CpuWeight, 6)],
            &current()
        ),
        Err(PlanDenial::DuplicateProperty(UnitProperty::CpuWeight))
    );
    assert_eq!(
        plan_properties(UNIT, &[(UnitProperty::TasksMax, 10)], &current()),
        Err(PlanDenial::CurrentValueUnknown(UnitProperty::TasksMax))
    );
}

#[test]
fn applying_writes_runtime_only_verifies_by_readback_and_restore_returns_the_originals() {
    let (mut client, calls, state) = systemd(&[("CPUWeight", 100), ("MemoryMax", UNSET)], false);
    let plan = plan_properties(
        UNIT,
        &[
            (UnitProperty::CpuWeight, 300),
            (UnitProperty::MemoryMax, 1 << 30),
        ],
        &current(),
    )
    .unwrap();
    let authorization = OfflineAuthorization::from_campaign("test");
    apply(&mut client, &plan, &authorization).unwrap();
    assert_eq!(state.lock().unwrap()["CPUWeight"], 300);
    assert_eq!(state.lock().unwrap()["MemoryMax"], 1 << 30);
    restore(&mut client, &plan).unwrap();
    assert_eq!(state.lock().unwrap()["CPUWeight"], 100);
    assert_eq!(state.lock().unwrap()["MemoryMax"], UNSET);
    let members: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.member.clone())
        .collect();
    assert_eq!(
        members.iter().filter(|m| *m == "SetUnitProperties").count(),
        2
    );
}

#[test]
fn a_change_that_does_not_read_back_is_reported_after_the_originals_are_written_back() {
    let (mut client, _calls, state) = systemd(&[("CPUWeight", 100)], true);
    let plan = plan_properties(UNIT, &[(UnitProperty::CpuWeight, 300)], &current()).unwrap();
    let error = apply(
        &mut client,
        &plan,
        &OfflineAuthorization::from_campaign("test"),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            PropertyError::NotInEffect {
                property: UnitProperty::CpuWeight,
                wanted: 300,
                found: Some(100),
                restored: true,
            }
        ),
        "{error:?}"
    );
    assert_eq!(state.lock().unwrap()["CPUWeight"], 100);
}

#[test]
fn the_property_array_is_encoded_with_struct_alignment_and_a_counted_length() {
    let mut body = Writer::default();
    body.u64_properties(&[("CPUWeight", 7), ("IOWeight", 9)]);
    let mut reader = fake_reader(&body.buf);
    let Value::Array(items) = reader.value("a(sv)").unwrap() else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[1],
        Value::Struct(vec![
            Value::Str("IOWeight".into()),
            Value::Variant(Box::new(Value::U64(9)))
        ])
    );
}
