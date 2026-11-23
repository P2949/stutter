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
    let witness = observe(pid, &[]).expect("our own process is readable");
    assert!(
        witness.modules.iter().any(|name| name.contains("libc")),
        "{:?}",
        witness.modules.len()
    );
    assert!(witness.devices.contains("/dev/null"));
    assert!(!witness.ancestors.is_empty());
    assert!(!witness.shows(Backend::Module("winewayland.drv")));
    drop(null);
    assert!(observe(u32::MAX - 1, &[]).is_none());
}

fn content_witness(mapped: Option<&str>, on: &str, off: &str) -> Value {
    json!({
        "modules": ["d3d12core.dll"], "devices": [], "ancestors": [],
        "mapped": mapped.map(|hash| json!({ "d3d12core.dll": [hash] })).unwrap_or(json!({})),
        "tool_on_sha256": on, "tool_off_sha256": off,
    })
}

fn content_record(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let (on, off) = ("a".repeat(64), "b".repeat(64));
    let attempts: Vec<Value> = schedule()
        .iter()
        .enumerate()
        .map(|(slot, treatment)| {
            let mapped = if *treatment == Treatment::On { &on } else { &off };
            json!({
                "slot": slot, "treatment": treatment, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
                "witness": content_witness(Some(mapped), &on, &off),
            })
        })
        .collect();
    let mut value = json!({ "schema": 1, "factor": "vkd3d_low_latency", "attempts": attempts });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check_content(bytes: &[u8]) -> Result<(), &'static str> {
    verify(
        bytes,
        ResearchFactor::Vkd3dLowLatency,
        &schedule(),
        &processes(),
    )
}

#[test]
fn a_factor_told_apart_by_content_is_proven_by_which_build_the_process_mapped() {
    let (on, off) = ("a".repeat(64), "b".repeat(64));
    assert_eq!(check_content(&content_record(|_| {})), Ok(()));
    let refused = |edit: &dyn Fn(&mut Value), why: &'static str| {
        assert_eq!(check_content(&content_record(edit)), Err(why));
    };
    // An On attempt that mapped the standard build, or an Off attempt that mapped the low-latency one.
    refused(
        &|v| v["attempts"][1]["witness"] = content_witness(Some(&off), &on, &off),
        "FeatureBackendAbsentWhereOn",
    );
    refused(
        &|v| v["attempts"][0]["witness"] = content_witness(Some(&on), &on, &off),
        "FeatureProofOffBuildNotShown",
    );
    // Nothing mapped under the watched name shows neither build: unproven, never assumed to be the baseline.
    refused(
        &|v| v["attempts"][0]["witness"] = content_witness(None, &on, &off),
        "FeatureProofOffBuildNotShown",
    );
    // Both builds mapped is not one build.
    refused(
        &|v| {
            v["attempts"][1]["witness"]["mapped"] =
                json!({ "d3d12core.dll": [on.clone(), off.clone()] })
        },
        "FeatureBackendAbsentWhereOn",
    );
    // The tool's two builds must be known, distinct, and the same on every attempt.
    refused(
        &|v| {
            v["attempts"][2]["witness"]
                .as_object_mut()
                .unwrap()
                .remove("tool_on_sha256");
        },
        "FeatureProofToolBuildsUnknown",
    );
    refused(
        &|v| v["attempts"][2]["witness"]["tool_off_sha256"] = json!("c".repeat(64)),
        "FeatureProofToolBuildsUnknown",
    );
    let same = "a".repeat(64);
    refused(
        &|v| {
            for attempt in v["attempts"].as_array_mut().unwrap() {
                attempt["witness"]["tool_off_sha256"] = json!(same.clone());
            }
        },
        "FeatureProofToolBuildsUnknown",
    );
}

#[test]
fn either_side_of_the_pulse_driver_proves_the_audio_backend_and_its_absence_proves_the_baseline() {
    let witness = |module: Option<&str>| json!({ "modules": module.into_iter().collect::<Vec<_>>(), "devices": [], "ancestors": [] });
    let record = |on_module: &str| {
        let attempts: Vec<Value> = schedule()
            .iter()
            .enumerate()
            .map(|(slot, treatment)| {
                let on = *treatment == Treatment::On;
                json!({ "slot": slot, "treatment": treatment, "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000,
                        "witness": witness(on.then_some(on_module)) })
            })
            .collect();
        serde_json::to_vec(&json!({ "schema": 1, "factor": "audio_backend", "attempts": attempts }))
            .unwrap()
    };
    for module in ["winepulse.so", "winepulse.drv"] {
        assert_eq!(
            verify(
                &record(module),
                ResearchFactor::AudioBackend,
                &schedule(),
                &processes()
            ),
            Ok(()),
            "{module}"
        );
    }
    // The pipewire driver is the baseline's, not the factor's.
    assert_eq!(
        verify(
            &record("winepipewire.so"),
            ResearchFactor::AudioBackend,
            &schedule(),
            &processes()
        ),
        Err("FeatureBackendAbsentWhereOn")
    );
}

#[test]
fn a_watched_mapped_file_is_hashed_from_the_kernels_view_of_the_process() {
    // This process maps its C library: watching its name hashes the file the kernel shows mapped (the test binary
    // itself is above the size bound a hashed mapping may have).
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let path = maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .find(|path| {
            path.rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("libc.so"))
        })
        .expect("the C library is mapped")
        .to_owned();
    let exe = std::path::PathBuf::from(&path);
    let name = exe
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    let witness = observe(std::process::id(), &[name.as_str()]).unwrap();
    let hashes = witness
        .mapped
        .get(&name)
        .expect("the watched name is mapped");
    assert!(hashes.contains(&stutter_core::hash::sha256_hex(
        &std::fs::read(&exe).unwrap()
    )));
    // A name nobody asked to watch is not hashed.
    assert!(observe(std::process::id(), &[]).unwrap().mapped.is_empty());
}
