use std::{
    collections::BTreeMap,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use super::*;
use crate::integrations::{
    campaign_receipt::CampaignScope,
    dbus::{
        DbusClient, Writer,
        fake::{Call, Reply, serve},
        fake_reader,
    },
};

const UNIT: &str = "stutter-helper.service";
const GENERATION: u64 = 777;

type State = Arc<Mutex<BTreeMap<String, u64>>>;

/// Behaviour switches for the fake systemd.
#[derive(Default, Clone)]
struct Knobs {
    /// The next `SetUnitProperties` is applied, then answers with an error (once).
    error_after_apply: Arc<AtomicBool>,
    /// Every `SetUnitProperties` is applied and then errors.
    always_error: Arc<AtomicBool>,
    /// A unit that refuses every change.
    ignore_sets: Arc<AtomicBool>,
    /// After a set, this many `Get` calls fail.
    fail_gets_after_set: Arc<AtomicUsize>,
    sets: Arc<AtomicUsize>,
}

struct Fake {
    client: DbusClient,
    calls: Arc<Mutex<Vec<Call>>>,
    state: State,
    knobs: Knobs,
}

fn systemd(initial: &[(&str, u64)]) -> Fake {
    let mut all: BTreeMap<String, u64> =
        initial.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
    all.insert(GENERATION_PROPERTY.to_owned(), GENERATION);
    let state: State = Arc::new(Mutex::new(all));
    let knobs = Knobs::default();
    let (ours, theirs) = UnixStream::pair().unwrap();
    let (shared, k) = (Arc::clone(&state), knobs.clone());
    let calls = serve(theirs, move |call| match call.member.as_str() {
        "GetUnit" => {
            let mut body = Writer::default();
            body.string("/org/freedesktop/systemd1/unit/stutter_2dhelper_2eservice");
            Reply::Body("o".to_owned(), body.buf)
        }
        "Get" => {
            if k.sets.load(Ordering::SeqCst) > 0 {
                let left = k.fail_gets_after_set.load(Ordering::SeqCst);
                if left > 0 {
                    k.fail_gets_after_set.store(left - 1, Ordering::SeqCst);
                    return Reply::Error("org.freedesktop.DBus.Error.NoReply");
                }
            }
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
            assert_eq!(reader.u32().unwrap(), 1, "a unit-file write was requested");
            let Value::Array(items) = reader.value("a(sv)").unwrap() else {
                panic!("not an array");
            };
            k.sets.fetch_add(1, Ordering::SeqCst);
            if !k.ignore_sets.load(Ordering::SeqCst) {
                for item in items {
                    let Value::Struct(parts) = item else { panic!() };
                    let (Value::Str(name), Value::Variant(inner)) = (&parts[0], &parts[1]) else {
                        panic!("{parts:?}")
                    };
                    let Value::U64(value) = **inner else { panic!() };
                    shared.lock().unwrap().insert(name.clone(), value);
                }
            }
            if k.error_after_apply.swap(false, Ordering::SeqCst)
                || k.always_error.load(Ordering::SeqCst)
            {
                return Reply::Error("org.freedesktop.DBus.Error.Failed");
            }
            Reply::Body(String::new(), Vec::new())
        }
        _ => Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    });
    Fake {
        client: DbusClient::from_stream(ours, 0).unwrap(),
        calls,
        state,
        knobs,
    }
}

fn auth() -> OfflineAuthorization {
    OfflineAuthorization::from_campaign("test", CampaignScope::UnitProperties)
}

fn baseline() -> Vec<(UnitProperty, u64)> {
    vec![
        (UnitProperty::CpuWeight, 100),
        (UnitProperty::MemoryMax, UNSET),
    ]
}

fn plan(changes: &[(UnitProperty, u64)]) -> PropertyPlan {
    plan_properties(UNIT, changes, &baseline(), Some(GENERATION)).unwrap()
}

fn two_changes() -> PropertyPlan {
    plan(&[
        (UnitProperty::CpuWeight, 300),
        (UnitProperty::MemoryMax, 1 << 30),
    ])
}

fn fake() -> Fake {
    systemd(&[("CPUWeight", 100), ("MemoryMax", UNSET)])
}

fn sets(fake: &Fake) -> usize {
    fake.knobs.sets.load(Ordering::SeqCst)
}

#[test]
fn a_plan_exists_only_for_an_owned_unit_with_bounded_distinct_values_known_originals_and_a_generation()
 {
    let change = [(UnitProperty::CpuWeight, 200)];
    let ok = plan_properties(UNIT, &change, &baseline(), Some(GENERATION)).unwrap();
    assert_eq!(ok.restore, [(UnitProperty::CpuWeight, 100)]);
    assert_eq!(ok.generation, GENERATION);
    for unit in [
        "sshd.service",
        "stutter-x.timer",
        "stutter-.service",
        "stutter-a/b.service",
        "stutter-x",
        "user.slice",
    ] {
        assert_eq!(
            plan_properties(unit, &change, &baseline(), Some(1)).unwrap_err(),
            PlanDenial::NotOwnedUnit,
            "{unit}"
        );
    }
    let denied = |changes: &[(UnitProperty, u64)], generation| {
        plan_properties(UNIT, changes, &baseline(), generation).unwrap_err()
    };
    assert_eq!(denied(&[], Some(1)), PlanDenial::EmptyChange);
    assert_eq!(denied(&change, None), PlanDenial::GenerationUnknown);
    assert_eq!(denied(&change, Some(0)), PlanDenial::GenerationUnknown);
    for (property, value) in [
        (UnitProperty::CpuWeight, 0),
        (UnitProperty::IoWeight, 10_001),
        (UnitProperty::MemoryMax, 0),
    ] {
        let error = plan_properties(UNIT, &[(property, value)], &baseline(), Some(1));
        let expected = if property == UnitProperty::IoWeight {
            PlanDenial::CurrentValueUnknown(property)
        } else {
            PlanDenial::BadValue(property)
        };
        // The value is checked before the baseline is looked up.
        assert_eq!(
            error.unwrap_err(),
            if value == 10_001 {
                PlanDenial::BadValue(property)
            } else {
                expected
            }
        );
    }
    assert_eq!(
        denied(
            &[(UnitProperty::CpuWeight, 5), (UnitProperty::CpuWeight, 6)],
            Some(1)
        ),
        PlanDenial::DuplicateProperty(UnitProperty::CpuWeight)
    );
    assert_eq!(
        denied(&[(UnitProperty::TasksMax, 10)], Some(1)),
        PlanDenial::CurrentValueUnknown(UnitProperty::TasksMax)
    );
}

#[test]
fn applying_writes_runtime_only_verifies_by_readback_and_restore_returns_the_originals() {
    let mut fake = fake();
    let plan = two_changes();
    apply(&mut fake.client, &plan, &auth()).unwrap();
    assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 300);
    assert_eq!(fake.state.lock().unwrap()["MemoryMax"], 1 << 30);
    restore(&mut fake.client, &plan, &auth()).unwrap();
    assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 100);
    assert_eq!(fake.state.lock().unwrap()["MemoryMax"], UNSET);
    assert_eq!(sets(&fake), 2);
}

#[test]
fn a_property_changed_since_planning_or_a_restarted_unit_is_refused_with_nothing_written() {
    let mut fake = fake();
    let plan = two_changes();
    fake.state.lock().unwrap().insert("CPUWeight".into(), 555);
    let error = apply(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(
            error,
            PropertyError::StaleBaseline {
                property: UnitProperty::CpuWeight,
                planned: 100,
                found: Some(555)
            }
        ),
        "{error:?}"
    );
    assert_eq!(sets(&fake), 0, "a stale baseline was overwritten");

    let mut fake = self::fake();
    fake.state
        .lock()
        .unwrap()
        .insert(GENERATION_PROPERTY.into(), GENERATION + 1);
    let error = apply(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(
            error,
            PropertyError::UnitChanged {
                planned: 777,
                found: Some(778)
            }
        ),
        "{error:?}"
    );
    assert_eq!(sets(&fake), 0, "a restarted unit was written to");
    let error = restore(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(error, PropertyError::UnitChanged { .. }),
        "{error:?}"
    );
    assert_eq!(sets(&fake), 0, "a restarted unit was restored over");
}

#[test]
fn a_write_that_errors_after_taking_effect_is_reconciled_back_to_the_baseline() {
    let mut fake = fake();
    fake.knobs.error_after_apply.store(true, Ordering::SeqCst);
    let plan = two_changes();
    let error = apply(&mut fake.client, &plan, &auth()).unwrap_err();
    // The call reported failure but both values were applied; both held our values, so both were
    // put back, and nothing is left unresolved.
    assert!(
        matches!(&error, PropertyError::PartialFailure { unresolved, .. } if unresolved.is_empty()),
        "{error:?}"
    );
    assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 100);
    assert_eq!(fake.state.lock().unwrap()["MemoryMax"], UNSET);
}

#[test]
fn when_the_reconcile_itself_fails_every_changed_property_is_reported_unresolved() {
    let mut fake = fake();
    fake.knobs.always_error.store(true, Ordering::SeqCst);
    let error = apply(&mut fake.client, &two_changes(), &auth()).unwrap_err();
    assert!(
        matches!(&error, PropertyError::PartialFailure { unresolved, .. } if unresolved.len() == 2),
        "{error:?}"
    );
}

#[test]
fn a_readback_failure_after_a_write_is_reconciled_and_not_left_applied() {
    let mut fake = fake();
    // The first read after the write fails; the reconcile reads succeed.
    fake.knobs.fail_gets_after_set.store(1, Ordering::SeqCst);
    let plan = two_changes();
    let error = apply(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(&error, PropertyError::PartialFailure { unresolved, .. } if unresolved.is_empty()),
        "{error:?}"
    );
    assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 100, "left applied");
    assert_eq!(fake.state.lock().unwrap()["MemoryMax"], UNSET);
}

#[test]
fn a_change_that_does_not_read_back_is_reported_with_nothing_unresolved() {
    let mut fake = fake();
    fake.knobs.ignore_sets.store(true, Ordering::SeqCst);
    let plan = two_changes();
    let error = apply(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(
            &error,
            PropertyError::NotInEffect {
                property: UnitProperty::CpuWeight,
                wanted: 300,
                found: Some(100),
                unresolved
            } if unresolved.is_empty()
        ),
        "{error:?}"
    );
}

#[test]
fn a_third_party_edit_after_apply_is_never_overwritten_by_restore() {
    let mut fake = fake();
    let plan = two_changes();
    apply(&mut fake.client, &plan, &auth()).unwrap();
    // Someone else moves CPUWeight; MemoryMax is still what this adapter wrote.
    fake.state.lock().unwrap().insert("CPUWeight".into(), 9_999);
    let error = restore(&mut fake.client, &plan, &auth()).unwrap_err();
    assert!(
        matches!(
            &error,
            PropertyError::ExternalChange { unresolved } if unresolved == &[UnitProperty::CpuWeight]
        ),
        "{error:?}"
    );
    assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 9_999, "clobbered");
    assert_eq!(
        fake.state.lock().unwrap()["MemoryMax"],
        UNSET,
        "our own value was restored"
    );
}

#[test]
fn unit_properties_need_their_own_scope_and_a_denial_writes_nothing() {
    for wrong in [
        CampaignScope::PowerProfileHold,
        CampaignScope::SchedulerSwitch,
    ] {
        let mut fake = fake();
        let plan = two_changes();
        let denied = OfflineAuthorization::from_campaign("test", wrong);
        let error = apply(&mut fake.client, &plan, &denied).unwrap_err();
        assert!(matches!(error, PropertyError::Unauthorized(_)), "{error:?}");
        assert!(
            matches!(
                restore(&mut fake.client, &plan, &denied),
                Err(PropertyError::Unauthorized(_))
            ),
            "restore ignored the scope"
        );
        assert!(
            fake.calls.lock().unwrap().is_empty(),
            "{wrong:?} reached the bus"
        );
        assert_eq!(fake.state.lock().unwrap()["CPUWeight"], 100);
    }
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
