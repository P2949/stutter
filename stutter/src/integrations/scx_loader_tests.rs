use std::{
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
};

use super::*;
use crate::integrations::{
    campaign_receipt::CampaignScope,
    dbus::{
        DbusClient, Writer,
        fake::{Call, Reply, serve},
    },
};

const XML: &str = r#"<node>
  <interface name="org.scx.Loader">
    <method name="StartScheduler">
      <arg type="s" direction="in"/>
      <arg type="u" direction="in"/>
    </method>
    <method name="SwitchScheduler">
      <arg type="s" direction="in"/>
      <arg type="u" direction="in"/>
    </method>
    <method name="StopScheduler"/>
    <property name="CurrentScheduler" type="s" access="read"/>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
  </interface>
</node>"#;

fn methods() -> BTreeMap<String, String> {
    method_signatures(XML, INTERFACE)
}

fn state(current: Option<&str>, mode: Option<u32>) -> ScxLoaderState {
    ScxLoaderState {
        current: current.map(str::to_owned),
        mode,
        supported: vec!["scx_lavd".into(), "scx_bpfland".into()],
    }
}

fn allow() -> Vec<String> {
    vec!["scx_lavd".into(), "scx_bpfland".into()]
}

#[test]
fn introspection_yields_each_methods_input_signature_for_this_interface_only() {
    let found = methods();
    assert_eq!(found["StartScheduler"], "su");
    assert_eq!(found["SwitchScheduler"], "su");
    assert_eq!(found["StopScheduler"], "");
    assert!(!found.contains_key("Ping"), "other interfaces are not read");
}

#[test]
fn a_switch_is_planned_only_to_an_allowlisted_supported_scheduler_with_an_exact_restore() {
    // Nothing running: start, and stopping restores the kernel default.
    let plan = plan_switch(&state(None, None), &methods(), "scx_lavd", 1, &allow()).unwrap();
    assert_eq!(
        (plan.method, plan.restore.clone()),
        ("StartScheduler", Restore::Stop)
    );
    // Something running: switch, and the restore is the prior scheduler and mode.
    let plan = plan_switch(
        &state(Some("scx_lavd"), Some(2)),
        &methods(),
        "scx_bpfland",
        1,
        &allow(),
    )
    .unwrap();
    assert_eq!(plan.method, "SwitchScheduler");
    assert_eq!(
        plan.restore,
        Restore::SwitchBack {
            name: "scx_lavd".into(),
            mode: 2
        }
    );
}

#[test]
fn every_unsafe_switch_has_its_own_typed_denial() {
    let none = state(None, None);
    assert_eq!(
        plan_switch(&none, &methods(), "scx_rustland", 1, &allow()),
        Err(SwitchDenial::NotAllowlisted)
    );
    let mut unsupported = none.clone();
    unsupported.supported = vec!["scx_lavd".into()];
    assert_eq!(
        plan_switch(&unsupported, &methods(), "scx_bpfland", 1, &allow()),
        Err(SwitchDenial::UnsupportedScheduler)
    );
    assert_eq!(
        plan_switch(
            &state(Some("scx_lavd"), Some(1)),
            &methods(),
            "scx_lavd",
            1,
            &allow()
        ),
        Err(SwitchDenial::AlreadyRunning)
    );
    let mut missing = methods();
    missing.remove("StartScheduler");
    assert_eq!(
        plan_switch(&none, &missing, "scx_lavd", 1, &allow()),
        Err(SwitchDenial::MethodMissing("StartScheduler"))
    );
    let mut odd = methods();
    odd.insert("SwitchScheduler".into(), "sus".into());
    assert_eq!(
        plan_switch(
            &state(Some("scx_lavd"), Some(1)),
            &odd,
            "scx_bpfland",
            1,
            &allow()
        ),
        Err(SwitchDenial::SignatureMismatch {
            method: "SwitchScheduler",
            found: "sus".into()
        })
    );
    // A running scheduler with an unknown mode cannot be restored exactly.
    assert_eq!(
        plan_switch(
            &state(Some("scx_lavd"), None),
            &methods(),
            "scx_bpfland",
            1,
            &allow()
        ),
        Err(SwitchDenial::UnsupportedRestore)
    );
    // Starting from nothing needs StopScheduler to be able to restore.
    let mut no_stop = methods();
    no_stop.remove("StopScheduler");
    assert_eq!(
        plan_switch(&none, &no_stop, "scx_lavd", 1, &allow()),
        Err(SwitchDenial::UnsupportedRestore)
    );
}

/// A fake `org.scx.Loader` whose state follows the calls it receives.
fn fake_loader() -> (DbusClient, crate::integrations::dbus::fake::Calls) {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    let live = Arc::new(Mutex::new((String::from("unknown"), 0u32)));
    let calls = serve(server_end, move |call: &Call| {
        let mut live = live.lock().unwrap();
        match (call.interface.as_str(), call.member.as_str()) {
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                let mut body = Writer::default();
                body.string(XML);
                Reply::Body("s".into(), body.buf)
            }
            ("org.freedesktop.DBus.Properties", "Get") => {
                let mut reader = crate::integrations::dbus::fake_reader(&call.body);
                let (_, property) = (reader.string(), reader.string());
                let mut body = Writer::default();
                match property.unwrap().as_str() {
                    "CurrentScheduler" => {
                        body.signature("s");
                        body.string(&live.0);
                    }
                    "SchedulerMode" => {
                        body.signature("u");
                        body.u32(live.1);
                    }
                    _ => {
                        body.signature("as");
                        body.u32(0);
                    }
                }
                Reply::Body("v".into(), body.buf)
            }
            (_, "StartScheduler" | "SwitchScheduler") => {
                let mut reader = crate::integrations::dbus::fake_reader(&call.body);
                live.0 = reader.string().unwrap();
                live.1 = reader.u32().unwrap();
                Reply::Body(String::new(), Vec::new())
            }
            (_, "StartSchedulerWithArgs" | "SwitchSchedulerWithArgs") => {
                let mut reader = crate::integrations::dbus::fake_reader(&call.body);
                live.0 = reader.string().unwrap();
                live.1 = 1;
                Reply::Body(String::new(), Vec::new())
            }
            (_, "StopScheduler") => {
                live.0 = "unknown".into();
                Reply::Body(String::new(), Vec::new())
            }
            _ => Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
        }
    });
    (DbusClient::from_stream(client_end, 1000).unwrap(), calls)
}

#[test]
fn an_authorized_switch_is_verified_and_restored_exactly_and_nothing_extra_is_sent() {
    let (mut client, calls) = fake_loader();
    assert_eq!(
        read_state(&mut client).unwrap().current,
        None,
        "unknown means no SCX scheduler"
    );
    let methods = introspect(&mut client).unwrap();
    let mut state = state(None, None);
    state.supported = vec!["scx_lavd".into()];
    let plan = plan_switch(&state, &methods, "scx_lavd", 1, &["scx_lavd".into()]).unwrap();
    let auth =
        OfflineAuthorization::from_campaign("scx-campaign-1", CampaignScope::SchedulerSwitch);
    assert_eq!(auth.campaign(), "scx-campaign-1");
    execute(&mut client, &plan, &auth, || Some("lavd".into())).unwrap();
    assert_eq!(
        read_state(&mut client).unwrap().current.as_deref(),
        Some("scx_lavd")
    );
    restore(&mut client, &plan, &auth).unwrap();
    assert_eq!(read_state(&mut client).unwrap().current, None);
    let writes: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| !matches!(call.member.as_str(), "Get" | "Introspect"))
        .map(|call| call.member.clone())
        .collect();
    assert_eq!(
        writes,
        vec!["StartScheduler", "StopScheduler"],
        "exactly the planned writes"
    );
}

#[test]
fn a_kernel_that_disagrees_makes_the_switch_not_in_effect() {
    let (mut client, _calls) = fake_loader();
    let methods = introspect(&mut client).unwrap();
    let mut state = state(None, None);
    state.supported = vec!["scx_lavd".into()];
    let plan = plan_switch(&state, &methods, "scx_lavd", 1, &["scx_lavd".into()]).unwrap();
    let auth = OfflineAuthorization::from_campaign("c", CampaignScope::SchedulerSwitch);
    let result = execute_within(
        &mut client,
        &plan,
        &auth,
        || Some("bpfland".into()),
        std::time::Duration::from_millis(120),
    );
    assert!(
        matches!(result, Err(ExecuteError::NotInEffect { .. })),
        "{result:?}"
    );
}

#[test]
fn missing_or_lookalike_kernel_ops_never_prove_a_scheduler_switch() {
    for ops in [None, Some("lavd_other"), Some("unrelated_lavd"), Some("")] {
        let (mut client, _) = fake_loader();
        let methods = introspect(&mut client).unwrap();
        let mut state = state(None, None);
        state.supported = vec!["scx_lavd".into()];
        let plan = plan_switch(&state, &methods, "scx_lavd", 1, &["scx_lavd".into()]).unwrap();
        let result = execute_within(
            &mut client,
            &plan,
            &OfflineAuthorization::from_campaign("fixture", CampaignScope::SchedulerSwitch),
            || ops.map(str::to_owned),
            std::time::Duration::from_millis(120),
        );
        assert!(
            matches!(result, Err(ExecuteError::NotInEffect { .. })),
            "{ops:?}: {result:?}"
        );
    }
}

fn arg_methods() -> BTreeMap<String, String> {
    let mut methods = methods();
    methods.insert("StartSchedulerWithArgs".into(), "sas".into());
    methods.insert("SwitchSchedulerWithArgs".into(), "sas".into());
    methods
}

fn reviewed() -> ArgAllowlist {
    vec![(
        "scx_lavd".into(),
        vec![vec!["--performance".into()], vec!["--powersave".into()]],
    )]
}

#[test]
fn only_a_reviewed_argument_list_can_be_planned_and_the_restore_is_exact() {
    let none = state(None, None);
    let args = vec!["--performance".to_owned()];
    let plan = plan_switch_with_args(
        &none,
        &arg_methods(),
        "scx_lavd",
        1,
        &args,
        &allow(),
        &reviewed(),
    )
    .unwrap();
    assert_eq!(plan.method, "StartSchedulerWithArgs");
    assert_eq!(plan.restore, Restore::Stop);
    assert_eq!(plan.args, args);

    let running = state(Some("scx_bpfland"), Some(2));
    let plan = plan_switch_with_args(
        &running,
        &arg_methods(),
        "scx_lavd",
        1,
        &args,
        &allow(),
        &reviewed(),
    )
    .unwrap();
    assert_eq!(plan.method, "SwitchSchedulerWithArgs");
    assert_eq!(
        plan.restore,
        Restore::SwitchBack {
            name: "scx_bpfland".into(),
            mode: 2
        }
    );

    let free_form = vec!["--perf; rm -rf /".to_owned()];
    assert_eq!(
        plan_switch_with_args(
            &none,
            &arg_methods(),
            "scx_lavd",
            1,
            &free_form,
            &allow(),
            &reviewed()
        ),
        Err(SwitchDenial::ArgumentsNotAllowlisted)
    );
    assert_eq!(
        plan_switch_with_args(
            &none,
            &arg_methods(),
            "scx_bpfland",
            1,
            &args,
            &allow(),
            &reviewed()
        ),
        Err(SwitchDenial::ArgumentsNotAllowlisted),
        "a list reviewed for one scheduler is not valid for another"
    );
    let mut no_method = arg_methods();
    no_method.remove("StartSchedulerWithArgs");
    assert_eq!(
        plan_switch_with_args(
            &none,
            &no_method,
            "scx_lavd",
            1,
            &args,
            &allow(),
            &reviewed()
        ),
        Err(SwitchDenial::MethodMissing("StartSchedulerWithArgs"))
    );
    let mut no_back = arg_methods();
    no_back.remove("SwitchScheduler");
    assert_eq!(
        plan_switch_with_args(
            &running,
            &no_back,
            "scx_lavd",
            1,
            &args,
            &allow(),
            &reviewed()
        ),
        Err(SwitchDenial::UnsupportedRestore),
        "a switch with arguments needs the plain switch to restore"
    );
    // No arguments is the plain switch.
    let plain = plan_switch_with_args(
        &none,
        &arg_methods(),
        "scx_lavd",
        1,
        &[],
        &allow(),
        &reviewed(),
    )
    .unwrap();
    assert_eq!(plain.method, "StartScheduler");
    assert!(plain.args.is_empty());
}

#[test]
fn a_string_array_is_encoded_with_its_byte_length_and_inter_element_padding() {
    let mut writer = Writer::default();
    writer.string_array(&["ab".to_owned(), "c".to_owned()]);
    // "ab": 4 + 2 + 1 = 7 bytes, 1 pad, "c": 4 + 1 + 1 = 6 bytes: 14 bytes of array data.
    assert_eq!(&writer.buf[..4], &14u32.to_le_bytes());
    assert_eq!(writer.buf.len(), 4 + 14);
    let mut empty = Writer::default();
    empty.string_array(&[]);
    assert_eq!(empty.buf, 0u32.to_le_bytes());
}

#[test]
fn an_authorized_switch_with_arguments_sends_exactly_the_reviewed_list() {
    let (mut client, calls) = fake_loader();
    let methods = {
        let mut found = introspect(&mut client).unwrap();
        found.insert("StartSchedulerWithArgs".into(), "sas".into());
        found
    };
    let mut state = state(None, None);
    state.supported = vec!["scx_lavd".into()];
    let args = vec!["--performance".to_owned()];
    let plan = plan_switch_with_args(
        &state,
        &methods,
        "scx_lavd",
        1,
        &args,
        &["scx_lavd".into()],
        &reviewed(),
    )
    .unwrap();
    let auth = OfflineAuthorization::from_campaign("c", CampaignScope::SchedulerSwitch);
    execute(&mut client, &plan, &auth, || Some("lavd".into())).unwrap();
    let sent = calls
        .lock()
        .unwrap()
        .iter()
        .find(|call| call.member == "StartSchedulerWithArgs")
        .cloned()
        .unwrap();
    assert_eq!(sent.signature, "sas");
    let mut reader = crate::integrations::dbus::fake_reader(&sent.body);
    assert_eq!(reader.string().unwrap(), "scx_lavd");
    assert_eq!(
        reader.value("as").unwrap(),
        crate::integrations::dbus::Value::Array(vec![crate::integrations::dbus::Value::Str(
            "--performance".into()
        )])
    );
    restore(&mut client, &plan, &auth).unwrap();
}

#[test]
fn an_authorization_for_another_scope_never_reaches_the_bus() {
    for wrong in [
        CampaignScope::PowerProfileHold,
        CampaignScope::UnitProperties,
    ] {
        let (mut client, calls) = fake_loader();
        let methods = introspect(&mut client).unwrap();
        let mut state = state(None, None);
        state.supported = vec!["scx_lavd".into()];
        let plan = plan_switch(&state, &methods, "scx_lavd", 1, &["scx_lavd".into()]).unwrap();
        let auth = OfflineAuthorization::from_campaign("c", wrong);
        let seen = calls.lock().unwrap().len();
        let result = execute(&mut client, &plan, &auth, || Some("lavd".into()));
        assert!(
            matches!(
                result,
                Err(ExecuteError::Unauthorized(
                    crate::integrations::campaign_receipt::AuthorizationDenied::WrongScope { .. }
                ))
            ),
            "{wrong:?}: {result:?}"
        );
        assert!(matches!(
            restore(&mut client, &plan, &auth),
            Err(ExecuteError::Unauthorized(_))
        ));
        assert_eq!(
            calls.lock().unwrap().len(),
            seen,
            "a denied authorization sent something to the bus"
        );
    }
}

/// Live: the real `scx_loader` on a *private* bus, through the production authorization factory.
///
/// Needs `STUTTER_LIVE_SCX_BUS` (the private bus socket path), `STUTTER_LIVE_SCX_RECEIPT` (a receipt
/// a root process wrote under a root-owned path, granting `scheduler_switch` to this user for this
/// boot) and `STUTTER_LIVE_SCX_JOURNAL` (a new file path for the local use journal). The system bus
/// is never touched; only the scheduler the test starts is switched in and then stopped.
#[test]
#[ignore = "live: needs a real scx_loader on a private bus and a root-written receipt"]
fn live_real_scx_loader_switch_and_restore_over_a_private_bus() {
    use crate::integrations::campaign_receipt::mint_authorization;
    let env = |name: &str| {
        std::env::var(name).unwrap_or_else(|_| panic!("BlockedResource: {name} is not set"))
    };
    let bus = env("STUTTER_LIVE_SCX_BUS");
    let receipt = env("STUTTER_LIVE_SCX_RECEIPT");
    let journal = env("STUTTER_LIVE_SCX_JOURNAL");
    let scheduler = std::env::var("STUTTER_LIVE_SCX_SCHEDULER").unwrap_or("scx_bpfland".into());

    let mut client =
        DbusClient::connect_path(std::path::Path::new(&bus), crate::syscall::geteuid())
            .expect("connect to the private bus");
    // The production factory: a root-written receipt, not a fixture constructor.
    let auth = mint_authorization(
        std::path::Path::new(&receipt),
        std::path::Path::new(&journal),
        CampaignScope::SchedulerSwitch,
    )
    .expect("the receipt must verify");
    assert_eq!(auth.scope(), CampaignScope::SchedulerSwitch);

    let before = read_state(&mut client).expect("a real scx_loader answers GetState-equivalents");
    assert_eq!(
        before.current, None,
        "a scheduler is already running: refusing to disturb it"
    );
    let methods = introspect(&mut client).expect("introspection of the real loader");
    let mut planning = state(None, None);
    planning.supported = before.supported.clone();
    assert!(
        planning.supported.contains(&scheduler),
        "the loader does not list {scheduler}: {:?}",
        planning.supported
    );
    let plan = plan_switch(
        &planning,
        &methods,
        &scheduler,
        0,
        std::slice::from_ref(&scheduler),
    )
    .expect("the real loader's method signatures match what the adapter expects");

    let kernel_ops = || {
        std::fs::read_to_string("/sys/kernel/sched_ext/root/ops")
            .ok()
            .map(|text| text.trim().to_owned())
    };
    if let Err(error) = execute(&mut client, &plan, &auth, kernel_ops) {
        // Report what the real loader and kernel say, so a mismatch names itself.
        let seen = read_state(&mut client);
        panic!(
            "switch not verified: {error:?}; planned mode {}; loader state {seen:?}; kernel ops {:?}; sched_ext state {:?}",
            plan.mode,
            std::fs::read_to_string("/sys/kernel/sched_ext/root/ops"),
            std::fs::read_to_string("/sys/kernel/sched_ext/state"),
        );
    }

    let during = read_state(&mut client).unwrap();
    assert_eq!(during.current.as_deref(), Some(scheduler.as_str()));
    let enabled = std::fs::read_to_string("/sys/kernel/sched_ext/state").unwrap();
    assert_eq!(enabled.trim(), "enabled");

    restore(&mut client, &plan, &auth).expect("restore verified");
    let after = read_state(&mut client).unwrap();
    assert_eq!(after.current, None, "the loader still reports a scheduler");
    // The kernel may take a moment to finish unloading.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string("/sys/kernel/sched_ext/state").unwrap();
        if text.trim() == "disabled" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sched_ext did not return to disabled"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn a_switch_the_kernel_attaches_a_moment_later_is_waited_for_and_one_it_never_attaches_is_not() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // The loader answers at once; the kernel reports the scheduler only on its third reading.
    let (mut client, _calls) = fake_loader();
    let methods = introspect(&mut client).unwrap();
    let mut planning = state(None, None);
    planning.supported = vec!["scx_lavd".into()];
    let plan = plan_switch(&planning, &methods, "scx_lavd", 1, &["scx_lavd".into()]).unwrap();
    let auth = OfflineAuthorization::from_campaign("late", CampaignScope::SchedulerSwitch);
    let reads = AtomicUsize::new(0);
    execute_within(
        &mut client,
        &plan,
        &auth,
        || (reads.fetch_add(1, Ordering::SeqCst) >= 2).then(|| "lavd".to_owned()),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert!(
        reads.load(Ordering::SeqCst) >= 3,
        "the kernel was read only once"
    );

    let (mut client, _calls) = fake_loader();
    let never = execute_within(
        &mut client,
        &plan,
        &auth,
        || None,
        std::time::Duration::from_millis(150),
    );
    assert!(
        matches!(never, Err(ExecuteError::NotInEffect { .. })),
        "{never:?}"
    );
}

#[test]
fn the_kernels_versioned_ops_name_matches_its_scheduler_and_lookalikes_do_not() {
    // What a real kernel reports for scx_bpfland 1.1.4.
    assert!(kernel_ops_match(
        "bpfland_1.1.4_x86_64_unknown_linux_gnu",
        "scx_bpfland"
    ));
    assert!(kernel_ops_match(
        "bpfland_1.1.4_x86_64_unknown_linux_gnu\n",
        "scx_bpfland"
    ));
    assert!(kernel_ops_match("bpfland", "scx_bpfland"));
    assert!(kernel_ops_match("scx_bpfland", "scx_bpfland"));
    assert!(kernel_ops_match("lavd", "scx_lavd"));
    for wrong in [
        "bpfland_other",
        "bpflandx",
        "xbpfland",
        "unrelated_bpfland",
        "lavd_1.0",
        "",
        "_1",
    ] {
        assert!(!kernel_ops_match(wrong, "scx_bpfland"), "{wrong:?}");
    }
    assert!(!kernel_ops_match("anything_1", ""));
    assert!(!kernel_ops_match("anything_1", "scx_"));
}
