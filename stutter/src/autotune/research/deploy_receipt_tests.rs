use std::os::unix::fs::PermissionsExt;

use super::*;

const NOW: u64 = 1_000_000;
const CONTROLLER: &str = "stutter-autotune-bounded@user.service";
const WORKER: &str = "stutter-privileged-worker@user.service";

fn store() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn binary(store: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::fs::canonicalize(store.path()).unwrap().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A directory shaped like `/proc`: `<pid>/exe` is a link to the executable that process runs.
fn proc_with(entries: &[(u32, &Path)]) -> tempfile::TempDir {
    let proc = tempfile::tempdir().unwrap();
    for (pid, exe) in entries {
        let dir = proc.path().join(pid.to_string());
        std::fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(exe, dir.join("exe")).unwrap();
        std::fs::write(
            dir.join("cmdline"),
            format!("{}\0autotune\0--mode\0x\0", exe.display()),
        )
        .unwrap();
        // pid (comm) state, then fields 4..21, then field 22 (starttime). The command name may hold
        // spaces and parentheses.
        std::fs::write(
            dir.join("stat"),
            format!("{pid} (stutter (x) y) S{} 777 0 0\n", " 0".repeat(18)),
        )
        .unwrap();
    }
    proc
}

fn shown(exec: &Path, state: &str, pid: u32) -> String {
    format!(
        "ActiveState={state}\nMainPID={pid}\nExecStart={{ path={} ; argv[]={} autotune --mode x ; ignore_errors=no }}\n",
        exec.display(),
        exec.display()
    )
}

fn sha(bytes: &[u8]) -> String {
    stutter_core::hash::sha256_hex(bytes)
}

/// A receipt for a healthy deployment of `bytes`: both required units active, configured and running
/// the same binary, taken in boot `b1` just before `NOW`.
fn healthy(bytes: &[u8]) -> (DeployReceipt, tempfile::TempDir, tempfile::TempDir) {
    let store = store();
    let exe = binary(&store, "pinned-stutter", bytes);
    let proc = proc_with(&[(10, &exe), (11, &exe)]);
    let units = vec![
        unit_receipt(
            CONTROLLER,
            &shown(&exe, "active", 10),
            store.path(),
            proc.path(),
        )
        .unwrap(),
        unit_receipt(
            WORKER,
            &shown(&exe, "active", 11),
            store.path(),
            proc.path(),
        )
        .unwrap(),
    ];
    (
        receipt(units, &sha(bytes), NOW - 100, Some("b1".into())),
        store,
        proc,
    )
}

#[test]
fn the_unit_binary_state_and_running_process_are_read_from_systemctl_and_proc() {
    let store = store();
    let exe = binary(&store, "h-stutter", b"release bytes");
    let proc = proc_with(&[(42, &exe)]);
    let text = shown(&exe, "active", 42);
    assert_eq!(exec_path(&text).as_deref(), exe.to_str());
    let unit = unit_receipt(CONTROLLER, &text, store.path(), proc.path()).unwrap();
    assert_eq!(unit.active_state, "active");
    assert_eq!(unit.main_pid, 42);
    assert_eq!(unit.binary_sha256, sha(b"release bytes"));
    assert_eq!(unit.running_binary_sha256, Some(sha(b"release bytes")));
    assert_eq!(unit.running_argv0.as_deref(), exe.to_str());
    assert_eq!(
        unit.start_ticks,
        Some(777),
        "starttime is read past a command name with parentheses"
    );
    assert_eq!(exec_path("ActiveState=active\n"), None);
    // A unit with no process has nothing running to attest.
    let idle = unit_receipt(
        CONTROLLER,
        &shown(&exe, "inactive", 0),
        store.path(),
        proc.path(),
    );
    assert_eq!(idle.unwrap().running_binary_sha256, None);
}

#[test]
fn a_binary_outside_the_store_or_unreadable_is_refused() {
    let store = store();
    let proc = proc_with(&[]);
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = std::fs::canonicalize(elsewhere.path())
        .unwrap()
        .join("stutter");
    std::fs::write(&outside, b"x").unwrap();
    // A mutable path is not an immutable store object.
    assert!(
        unit_receipt(
            "u",
            &shown(&outside, "active", 1),
            store.path(),
            proc.path()
        )
        .is_err()
    );
    assert!(unit_receipt("u", "ActiveState=active\n", store.path(), proc.path()).is_err());
    // A symlink out of the store is refused too.
    let inside = std::fs::canonicalize(store.path()).unwrap().join("link");
    std::os::unix::fs::symlink(&outside, &inside).unwrap();
    assert!(binary_sha256(&inside, store.path()).is_err());
}

#[test]
fn a_configured_binary_is_not_the_running_one() {
    // The unit is configured with the pinned binary, but its process is running another: a stale
    // process kept alive across a switch.
    let store = store();
    let pinned = binary(&store, "a-stutter", b"pinned");
    let old = binary(&store, "b-stutter", b"older build");
    let proc = proc_with(&[(10, &old), (11, &pinned)]);
    let controller = unit_receipt(
        CONTROLLER,
        &shown(&pinned, "active", 10),
        store.path(),
        proc.path(),
    )
    .unwrap();
    let worker = unit_receipt(
        WORKER,
        &shown(&pinned, "active", 11),
        store.path(),
        proc.path(),
    )
    .unwrap();
    assert_eq!(controller.binary_sha256, sha(b"pinned"));
    assert_eq!(controller.running_binary_sha256, Some(sha(b"older build")));
    let made = receipt(
        vec![controller, worker],
        &sha(b"pinned"),
        NOW,
        Some("b1".into()),
    );
    assert!(!made.binary_matches_pin);
    assert_eq!(
        attested(&made, &sha(b"pinned"), Some("b1"), NOW, bound()),
        Err("UnitNotRunningThisBinary")
    );
}

#[test]
fn a_healthy_receipt_attests_and_every_way_it_can_be_wrong_is_its_own_refusal() {
    let me = sha(b"verifier");
    let (good, _store, _proc) = healthy(b"verifier");
    assert_eq!(attested(&good, &me, Some("b1"), NOW, bound()), Ok(()));

    let refused = |edit: &dyn Fn(&mut DeployReceipt), expected: &'static str| {
        let mut bad = good.clone();
        edit(&mut bad);
        assert_eq!(attested(&bad, &me, Some("b1"), NOW, bound()), Err(expected));
    };
    refused(&|r| r.schema = 1, "ReceiptSchemaTooOld");
    refused(
        &|r| r.expected_binary_sha256 = "0".repeat(64),
        "ReceiptPinIsNotThisBinary",
    );
    refused(&|r| r.units.clear(), "RequiredUnitMissing");
    refused(&|r| drop(r.units.pop()), "RequiredUnitMissing");
    refused(
        &|r| {
            let again = r.units[0].clone();
            r.units.push(again);
        },
        "RequiredUnitDuplicated",
    );
    refused(
        &|r| {
            let mut extra = r.units[0].clone();
            extra.unit = "stutter-other@user.service".into();
            r.units.push(extra);
        },
        "UnexpectedUnit",
    );
    refused(
        &|r| r.units[1].active_state = "failed".into(),
        "UnitNotActive",
    );
    refused(&|r| r.units[0].main_pid = 0, "UnitNotRunningThisBinary");
    refused(
        &|r| r.units[1].binary_sha256 = "1".repeat(64),
        "UnitNotRunningThisBinary",
    );
    refused(&|r| r.boot_id = None, "ReceiptFromAnotherBoot");
    refused(&|r| r.boot_id = Some("b0".into()), "ReceiptFromAnotherBoot");
    refused(
        &|r| r.captured_unix = NOW - MAX_RECEIPT_AGE_SECONDS - 1,
        "ReceiptNotFresh",
    );
    refused(&|r| r.captured_unix = NOW + 3600, "ReceiptNotFresh");
    // Another boot as seen now, or an unreadable one.
    assert_eq!(
        attested(&good, &me, Some("b2"), NOW, bound()),
        Err("ReceiptFromAnotherBoot")
    );
    assert_eq!(
        attested(&good, &me, None, NOW, bound()),
        Err("ReceiptFromAnotherBoot")
    );
    // A receipt about some other binary says nothing about the one doing the verifying.
    assert_eq!(
        attested(&good, &"2".repeat(64), Some("b1"), NOW, bound()),
        Err("ReceiptPinIsNotThisBinary")
    );
}

#[test]
fn stored_summary_flags_are_never_trusted() {
    let me = sha(b"verifier");
    let (mut lying, _store, _proc) = healthy(b"verifier");
    lying.units.clear();
    lying.binary_matches_pin = true;
    lying.units_active = true;
    assert_eq!(
        attested(&lying, &me, Some("b1"), NOW, bound()),
        Err("RequiredUnitMissing"),
        "an empty unit list with both flags set true is not a deployment"
    );
}

#[test]
fn g8_reads_the_newest_receipt_from_the_study_directory() {
    let me = sha(b"verifier");
    let (good, _store, _proc) = healthy(b"verifier");
    let study = tempfile::tempdir().unwrap();
    let write = |name: &str, receipt: &DeployReceipt| {
        let path = study.path().join(name);
        std::fs::write(&path, serde_json::to_vec(receipt).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    // No receipt: nothing attested.
    assert!(!deployment_attested(
        study.path(),
        &me,
        Some("b1"),
        NOW,
        bound()
    ));
    write("deploy-receipt-1.json", &good);
    assert!(deployment_attested(
        study.path(),
        &me,
        Some("b1"),
        NOW,
        bound()
    ));
    // The newest receipt decides: a later one that no longer attests withdraws it.
    let mut later = good.clone();
    later.captured_unix = NOW - 50;
    later.units[0].active_state = "inactive".into();
    write("deploy-receipt-2.json", &later);
    assert!(!deployment_attested(
        study.path(),
        &me,
        Some("b1"),
        NOW,
        bound()
    ));
}

#[test]
fn unit_names_are_validated_before_anything_runs() {
    assert!(show(Path::new("/nonexistent"), "bad name; rm").is_err());
    assert!(show(Path::new("/nonexistent"), "").is_err());
}

#[test]
fn where_the_executable_is_unreadable_the_process_must_report_the_units_own_store_path() {
    let me = sha(b"verifier");
    let (good, _store, _proc) = healthy(b"verifier");
    // The service processes are not readable by the user: no executable hash can be taken.
    let mut unprivileged = good.clone();
    for unit in &mut unprivileged.units {
        unit.running_binary_sha256 = None;
    }
    assert_eq!(
        attested(&unprivileged, &me, Some("b1"), NOW, bound()),
        Ok(())
    );
    // Then it must name this unit's own immutable path, and be an identified process.
    let mut elsewhere = unprivileged.clone();
    elsewhere.units[0].running_argv0 = Some("/nix/store/other-stutter".into());
    assert_eq!(
        attested(&elsewhere, &me, Some("b1"), NOW, bound()),
        Err("UnitNotRunningThisBinary")
    );
    let mut unnamed = unprivileged.clone();
    unnamed.units[1].running_argv0 = None;
    assert_eq!(
        attested(&unnamed, &me, Some("b1"), NOW, bound()),
        Err("UnitNotRunningThisBinary")
    );
    let mut unidentified = unprivileged;
    unidentified.units[0].start_ticks = None;
    assert_eq!(
        attested(&unidentified, &me, Some("b1"), NOW, bound()),
        Err("UnitNotRunningThisBinary")
    );
    // A readable executable that differs is refused whatever the process says about itself.
    let mut liar = good;
    liar.units[0].running_binary_sha256 = Some("3".repeat(64));
    assert_eq!(
        attested(&liar, &me, Some("b1"), NOW, bound()),
        Err("UnitNotRunningThisBinary")
    );
}

/// The source identity of a campaign frozen by the "verifier" build at revision "rev".
fn campaign() -> &'static str {
    static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(|| {
        crate::autotune::research::host::source_identity_of("rev", &sha(b"verifier")).unwrap()
    })
}

fn bound() -> Option<CampaignBinding<'static>> {
    Some(CampaignBinding {
        revision: "rev",
        campaign_source_sha256: campaign(),
    })
}

#[test]
fn the_deployed_binary_must_be_the_build_that_froze_the_campaign() {
    let me = sha(b"verifier");
    let (good, _store, _proc) = healthy(b"verifier");
    assert_eq!(attested(&good, &me, Some("b1"), NOW, bound()), Ok(()));
    // No campaign to bind to: a deployment is not attested in the abstract.
    assert_eq!(
        attested(&good, &me, Some("b1"), NOW, None),
        Err("ReceiptNotBoundToACampaign")
    );
    // A campaign frozen by another build (another binary, or another revision of the same one).
    let other_binary =
        crate::autotune::research::host::source_identity_of("rev", &sha(b"older build")).unwrap();
    let other_revision = crate::autotune::research::host::source_identity_of("rev2", &me).unwrap();
    for frozen in [other_binary.as_str(), other_revision.as_str(), "0000"] {
        let binding = CampaignBinding {
            revision: "rev",
            campaign_source_sha256: frozen,
        };
        assert_eq!(
            attested(&good, &me, Some("b1"), NOW, Some(binding)),
            Err("DeployedBinaryIsNotTheCampaignsBuild")
        );
    }
}

#[test]
fn a_present_malformed_deploy_receipt_attests_nothing_instead_of_falling_back_to_an_older_one() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_latest(dir.path()).is_none());
    std::fs::write(dir.path().join("deploy-receipt-9.json"), b"{ no").unwrap();
    assert!(read_latest(dir.path()).is_none());
}
