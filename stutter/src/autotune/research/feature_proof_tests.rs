use serde_json::{Value, json};

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..4)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000,
            completed_unix: 2_000,
        })
        .collect()
}

fn schedule() -> Vec<Treatment> {
    vec![Treatment::Off, Treatment::On, Treatment::On, Treatment::Off]
}

fn witness(module: Option<&str>) -> Value {
    json!({
        "modules": module.into_iter().collect::<Vec<_>>(), "devices": ["/dev/null"], "ancestors": ["steam"],
    })
}

fn record(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let attempts: Vec<Value> = schedule()
        .iter()
        .enumerate()
        .map(|(slot, treatment)| {
            let on = *treatment == Treatment::On;
            json!({
                "slot": slot, "treatment": treatment, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
                "witness": witness(on.then_some("winewayland.drv")),
            })
        })
        .collect();
    let mut value = json!({ "schema": 1, "factor": "wine_wayland", "attempts": attempts });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check(bytes: &[u8]) -> Result<(), &'static str> {
    verify(
        bytes,
        ResearchFactor::WineWayland,
        &schedule(),
        &processes(),
    )
}

#[test]
fn the_backend_must_be_present_on_every_on_attempt_and_absent_on_every_off_attempt() {
    assert_eq!(check(&record(|_| {})), Ok(()));
    let refused = |edit: &dyn Fn(&mut Value), why: &'static str| {
        assert_eq!(check(&record(edit)), Err(why));
    };
    refused(
        &|v| v["attempts"][1]["witness"] = witness(None),
        "FeatureBackendAbsentWhereOn",
    );
    refused(
        &|v| v["attempts"][0]["witness"] = witness(Some("winewayland.drv")),
        "FeatureBackendPresentWhereOff",
    );
    refused(
        &|v| v["factor"] = "nt_sync".into(),
        "FeatureProofForAnotherFactor",
    );
    refused(
        &|v| {
            v["attempts"].as_array_mut().unwrap().pop();
        },
        "FeatureProofNotEveryAttempt",
    );
    refused(
        &|v| v["attempts"][2]["pid"] = 999.into(),
        "FeatureProofForAnotherProcess",
    );
    refused(
        &|v| v["attempts"][2]["treatment"] = "off".into(),
        "FeatureProofTreatmentMismatch",
    );
    assert_eq!(check(b"{"), Err("FeatureProofUnreadable"));
    // A factor whose backend is not modelled is never proven, whatever the record says.
    assert_eq!(
        verify(
            &record(|_| {}),
            ResearchFactor::Hdr,
            &schedule(),
            &processes()
        ),
        Err("FeatureBackendNotModelled")
    );
    // One treatment only proves nothing about the difference.
    let on_only = vec![Treatment::On; 4];
    let all_on: Vec<Value> = (0..4).map(|s| json!({ "slot": s, "treatment": "on", "pid": 100 + s, "start_ticks": 50 + s, "uid": 1000, "witness": witness(Some("winewayland.drv")) })).collect();
    let bytes =
        serde_json::to_vec(&json!({ "schema": 1, "factor": "wine_wayland", "attempts": all_on }))
            .unwrap();
    assert_eq!(
        verify(&bytes, ResearchFactor::WineWayland, &on_only, &processes()),
        Err("FeatureProofNeedsBothTreatments")
    );
}

#[test]
fn the_witness_is_read_from_the_kernels_view_of_a_real_process() {
    // This very process: it maps its own executable and libc, holds /dev/null open when it opens it, and has
    // ancestors; a process that does not exist cannot be read at all.
    let pid = std::process::id();
    let null = std::fs::File::open("/dev/null").unwrap();
    let witness = observe(pid).expect("our own process is readable");
    assert!(
        witness.modules.iter().any(|name| name.contains("libc")),
        "{:?}",
        witness.modules.len()
    );
    assert!(witness.devices.contains("/dev/null"));
    assert!(!witness.ancestors.is_empty());
    assert!(!witness.shows(Backend::Module("winewayland.drv")));
    drop(null);
    assert!(observe(u32::MAX - 1).is_none());
}
