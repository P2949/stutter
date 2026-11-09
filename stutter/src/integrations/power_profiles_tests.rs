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
    scx_loader::method_signatures,
};

const XML: &str = r#"<node>
  <interface name="org.freedesktop.UPower.PowerProfiles">
    <method name="HoldProfile">
      <arg type="s" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="ReleaseProfile">
      <arg type="u" direction="in"/>
    </method>
  </interface>
</node>"#;

fn methods() -> BTreeMap<String, String> {
    method_signatures(XML, INTERFACE)
}

fn allow() -> Vec<String> {
    vec!["performance".into(), "balanced".into()]
}

fn plan(profile: &str, active: Option<&str>) -> Result<HoldPlan, HoldDenial> {
    plan_hold(
        active,
        &methods(),
        profile,
        "autotune A/B",
        "stutter",
        &allow(),
    )
}

#[test]
fn a_hold_is_planned_only_for_an_allowlisted_known_profile_with_the_exact_methods() {
    let ok = plan("performance", Some("balanced")).unwrap();
    assert_eq!(ok.restore_to, "balanced");
    assert_eq!(
        plan("turbo", Some("balanced")),
        Err(HoldDenial::UnknownProfile)
    );
    assert_eq!(
        plan("power-saver", Some("balanced")),
        Err(HoldDenial::NotAllowlisted)
    );
    assert_eq!(
        plan("performance", Some("performance")),
        Err(HoldDenial::AlreadyActive)
    );
    assert_eq!(
        plan("performance", None),
        Err(HoldDenial::ActiveProfileUnknown)
    );
    let mut wrong = methods();
    wrong.insert("HoldProfile".into(), "ss".into());
    assert!(matches!(
        plan_hold(Some("balanced"), &wrong, "performance", "r", "a", &allow()),
        Err(HoldDenial::SignatureMismatch {
            method: "HoldProfile",
            ..
        })
    ));
    wrong.remove("ReleaseProfile");
    wrong.insert("HoldProfile".into(), "sss".into());
    assert_eq!(
        plan_hold(Some("balanced"), &wrong, "performance", "r", "a", &allow()),
        Err(HoldDenial::MethodMissing("ReleaseProfile"))
    );
}

fn string_variant(text: &str) -> Reply {
    let mut body = Writer::default();
    body.signature("s");
    body.string(text);
    Reply::Body("v".to_owned(), body.buf)
}

/// A fake daemon: the active profile is `balanced` until a hold is taken, then `held`, and a
/// release returns it to `after_release`.
fn daemon(held: &'static str, after_release: &'static str) -> (DbusClient, Arc<Mutex<Vec<Call>>>) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let active = Arc::new(Mutex::new("balanced".to_owned()));
    let calls = serve(theirs, move |call| match call.member.as_str() {
        "HoldProfile" => {
            *active.lock().unwrap() = held.to_owned();
            let mut body = Writer::default();
            body.u32(77);
            Reply::Body("u".to_owned(), body.buf)
        }
        "ReleaseProfile" => {
            *active.lock().unwrap() = after_release.to_owned();
            Reply::Body(String::new(), Vec::new())
        }
        "Get" => string_variant(&active.lock().unwrap()),
        _ => Reply::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    });
    (DbusClient::from_stream(ours, 1000).unwrap(), calls)
}

#[test]
fn a_hold_is_taken_verified_and_released_with_the_prior_profile_back() {
    let (mut client, calls) = daemon("performance", "balanced");
    let authorization =
        OfflineAuthorization::from_campaign("test", CampaignScope::PowerProfileHold);
    let hold = acquire(
        &mut client,
        &plan("performance", Some("balanced")).unwrap(),
        &authorization,
    )
    .unwrap();
    assert_eq!(hold.cookie(), 77);
    hold.release(&mut client).unwrap();
    let members: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.member.clone())
        .collect();
    assert_eq!(members, ["HoldProfile", "Get", "ReleaseProfile", "Get"]);
}

#[test]
fn a_hold_that_is_not_in_effect_is_released_at_once_and_a_failed_restore_is_reported() {
    let (mut client, calls) = daemon("balanced", "balanced");
    let authorization =
        OfflineAuthorization::from_campaign("test", CampaignScope::PowerProfileHold);
    let error = acquire(
        &mut client,
        &plan("performance", Some("balanced")).unwrap(),
        &authorization,
    )
    .unwrap_err();
    assert!(matches!(error, HoldError::NotInEffect { .. }));
    let members: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.member.clone())
        .collect();
    assert_eq!(members, ["HoldProfile", "Get", "ReleaseProfile"]);

    let (mut client, _) = daemon("performance", "power-saver");
    let hold = acquire(
        &mut client,
        &plan("performance", Some("balanced")).unwrap(),
        &authorization,
    )
    .unwrap();
    assert!(matches!(
        hold.release(&mut client),
        Err(HoldError::NotRestored { wanted, active }) if wanted == "balanced" && active == "power-saver"
    ));
}

#[test]
fn a_hold_with_another_scopes_authorization_is_denied_before_any_bus_call() {
    for wrong in [
        CampaignScope::SchedulerSwitch,
        CampaignScope::UnitProperties,
    ] {
        let (mut client, calls) = daemon("performance", "balanced");
        let before = calls.lock().unwrap().len();
        let error = acquire(
            &mut client,
            &plan("performance", Some("balanced")).unwrap(),
            &OfflineAuthorization::from_campaign("test", wrong),
        )
        .unwrap_err();
        assert!(matches!(error, HoldError::Unauthorized(_)), "{error:?}");
        assert_eq!(
            calls.lock().unwrap().len(),
            before,
            "{wrong:?} reached the bus"
        );
    }
}
