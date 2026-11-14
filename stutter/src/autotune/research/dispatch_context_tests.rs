use std::os::unix::fs::PermissionsExt;

use super::*;

fn fixture() -> (
    tempfile::TempDir,
    std::fs::File,
    ResearchIntentV1,
    AttemptSlot,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = std::fs::File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
    let mut intent = super::super::tests::intent();
    intent.uid = crate::syscall::geteuid();
    let slot = AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: intent.tools[0].source,
    };
    (dir, file, intent, slot)
}

fn env() -> BTreeMap<String, String> {
    [
        ("SteamAppId", "1091500"),
        ("STEAM_COMPAT_DATA_PATH", "/study/compatdata"),
        ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/home/u/.steam/steam"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .into()
}

fn spec() -> ArmSpec {
    ArmSpec {
        tool_dir: "/tools/GE".into(),
        study_compat_data: "/study/compatdata".into(),
        arm_env: BTreeMap::new(),
        steamapps: "/steam/steamapps".into(),
        proton_manifest: "/m.json".into(),
    }
}

fn argv(verb: &str) -> Vec<String> {
    ["/x/proton", verb, "/g/Cyberpunk2077.exe"]
        .map(str::to_owned)
        .to_vec()
}

fn armed() -> (tempfile::TempDir, std::fs::File, u64) {
    let (dir, file, intent, slot) = fixture();
    let now = intent.issued_unix_seconds + 1;
    arm_attempt(&file, &intent, &slot, spec(), (now, 60)).unwrap();
    mark_launch_requested(&file).unwrap();
    (dir, file, now + 1)
}

#[test]
fn steam_client_restart_update_or_missing_identity_refuses_dispatch() {
    let (_dir, _file, intent, slot) = fixture();
    let context = DispatchContextV1 {
        schema_version: 1,
        intent,
        slot,
        tickets: BTreeMap::new(),
        spec: spec(),
    };
    let frozen = context.intent.steam_client.clone();
    assert!(verify_frozen_steam_client(&context, Some(&frozen)).is_ok());
    assert!(verify_frozen_steam_client(&context, None).is_err());
    let mut restarted = frozen.clone();
    restarted.process.as_mut().unwrap().steam_start_ticks += 1;
    assert!(verify_frozen_steam_client(&context, Some(&restarted)).is_err());
    let mut updated = frozen.clone();
    updated.steamrt64_build_id += 1;
    assert!(verify_frozen_steam_client(&context, Some(&updated)).is_err());
    let mut absent = frozen;
    absent.process = None;
    assert!(verify_frozen_steam_client(&context, Some(&absent)).is_err());
}

#[test]
fn dispatcher_start_records_live_process_and_rejects_duplicate_or_wrong_boot() {
    let (_dir, file, mut intent, slot) = fixture();
    intent.boot_id = crate::telemetry::identity::read_boot_id().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    intent.issued_unix_seconds = now - 1;
    intent.deadline_unix_seconds = now + 3_600;
    arm_attempt(&file, &intent, &slot, spec(), (now, 60)).unwrap();
    mark_launch_requested(&file).unwrap();
    let (_, ticket) =
        authorize_dispatch(&file, &argv("run"), &env(), intent.uid, now + 1, &|_, _| {
            Ok(())
        })
        .unwrap();
    let mut wrong = ticket.clone();
    wrong.boot_id = "another-boot".into();
    assert!(record_dispatcher_start(&file, &wrong).is_err());
    let mut expired = ticket.clone();
    expired.expires_unix_seconds = now;
    assert!(record_dispatcher_start(&file, &expired).is_err());
    let record = record_dispatcher_start(&file, &ticket).unwrap();
    assert_eq!(record.pid.as_u32(), std::process::id());
    assert_eq!(record.uid, intent.uid);
    assert_eq!(record.ticket_id, ticket.ticket_id);
    assert_ne!(record.start_ticks, 0);
    assert_ne!(record.pid_namespace_inode, 0);
    assert!(record_dispatcher_start(&file, &ticket).is_err());
}

#[test]
fn host_anchor_requires_ticket_namespace_live_identity_and_steam_ancestry() {
    let (dir, file, mut intent, slot) = fixture();
    let steam = intent.steam_client.process.as_mut().unwrap();
    steam.steam_pid = 200;
    steam.steam_start_ticks = 40;
    steam.webhelper_pid = 201;
    steam.webhelper_start_ticks = 41;
    let now = intent.issued_unix_seconds + 1;
    let context = arm_attempt(&file, &intent, &slot, spec(), (now, 60)).unwrap();
    mark_launch_requested(&file).unwrap();
    let (_, ticket) =
        authorize_dispatch(&file, &argv("run"), &env(), intent.uid, now + 1, &|_, _| {
            Ok(())
        })
        .unwrap();
    let proc = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(proc.path().join("sys/kernel/random")).unwrap();
    std::fs::write(
        proc.path().join("sys/kernel/random/boot_id"),
        &intent.boot_id,
    )
    .unwrap();
    for pid in [123, 200, 201] {
        std::fs::create_dir_all(proc.path().join(pid.to_string()).join("ns")).unwrap();
    }
    std::fs::create_dir_all(proc.path().join("self/ns")).unwrap();
    let ns = proc.path().join("self/ns/pid");
    std::fs::write(&ns, b"namespace").unwrap();
    std::fs::hard_link(&ns, proc.path().join("123/ns/pid")).unwrap();
    let stat = |pid: u32, parent: u32, ticks: u64| {
        format!("{pid} (fixture) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0")
    };
    std::fs::write(proc.path().join("200/stat"), stat(200, 1, 40)).unwrap();
    std::fs::write(proc.path().join("201/stat"), stat(201, 200, 41)).unwrap();
    std::fs::write(proc.path().join("123/stat"), stat(123, 200, 80)).unwrap();
    let meta = std::fs::metadata(&ns).unwrap();
    let record = DispatcherStartV1 {
        schema_version: 1,
        ticket_id: ticket.ticket_id.clone(),
        intent_sha256: ticket.intent_sha256.clone(),
        attempt_sha256: ticket.attempt_sha256.clone(),
        verb: SteamVerb::Run,
        uid: intent.uid,
        boot_id: intent.boot_id.clone(),
        observed_unix_seconds: now + 1,
        pid: stutter_core::ids::Pid::new(123),
        start_ticks: 80,
        pid_namespace_device: meta.dev(),
        pid_namespace_inode: meta.ino(),
    };
    super::super::report_store::write_new(
        &file,
        "dispatcher-run.json",
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    assert_eq!(
        inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).unwrap(),
        Some(record)
    );
    let marker = dir.path().join(format!("used-{}", ticket.ticket_id));
    std::fs::write(proc.path().join("sys/kernel/random/boot_id"), "wrong-boot").unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::write(
        proc.path().join("sys/kernel/random/boot_id"),
        &intent.boot_id,
    )
    .unwrap();
    let original_marker = std::fs::read(&marker).unwrap();
    std::fs::write(&marker, b"tampered").unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::write(&marker, original_marker).unwrap();
    let process_ns = proc.path().join("123/ns/pid");
    std::fs::remove_file(&process_ns).unwrap();
    std::fs::write(&process_ns, b"other namespace").unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::remove_file(&process_ns).unwrap();
    std::fs::hard_link(&ns, &process_ns).unwrap();
    std::fs::write(proc.path().join("201/stat"), stat(201, 200, 42)).unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::write(proc.path().join("201/stat"), stat(201, 200, 41)).unwrap();
    std::fs::write(proc.path().join("123/stat"), stat(123, 200, 81)).unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::write(proc.path().join("123/stat"), stat(123, 1, 80)).unwrap();
    assert!(inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).is_err());
    std::fs::remove_dir_all(proc.path().join("123")).unwrap();
    assert_eq!(
        inspect_dispatcher_start(&file, &context, SteamVerb::Run, proc.path()).unwrap(),
        None
    );
}

#[test]
fn each_steam_verb_runs_once_and_replays_or_unknown_verbs_are_refused() {
    let (_dir, file, now) = armed();
    let euid = crate::syscall::geteuid();
    let wait = authorize_dispatch(
        &file,
        &argv("waitforexitandrun"),
        &env(),
        euid,
        now,
        &|_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(wait.0.program, std::path::Path::new("/tools/GE/proton"));
    assert!(
        authorize_dispatch(
            &file,
            &argv("waitforexitandrun"),
            &env(),
            euid,
            now,
            &|_, _| Ok(())
        )
        .is_err()
    );
    authorize_dispatch(&file, &argv("run"), &env(), euid, now, &|_, _| Ok(())).unwrap();
    assert!(authorize_dispatch(&file, &argv("run"), &env(), euid, now, &|_, _| Ok(())).is_err());
    assert!(
        authorize_dispatch(
            &file,
            &argv("getcompatpath"),
            &env(),
            euid,
            now,
            &|_, _| Ok(())
        )
        .is_err()
    );
}

#[test]
fn a_malformed_call_does_not_burn_the_ticket_and_other_users_or_expiry_are_refused() {
    let (_dir, file, now) = armed();
    let euid = crate::syscall::geteuid();
    let mut bad = env();
    bad.insert("SteamAppId".into(), "220".into());
    assert!(authorize_dispatch(&file, &argv("run"), &bad, euid, now, &|_, _| Ok(())).is_err());
    assert!(
        authorize_dispatch(&file, &argv("run"), &env(), euid, now + 600, &|_, _| Ok(())).is_err()
    );
    assert!(
        authorize_dispatch(&file, &argv("run"), &env(), euid + 1, now, &|_, _| Ok(())).is_err()
    );
    assert!(authorize_dispatch(&file, &argv("run"), &env(), euid, now, &|_, _| Ok(())).is_ok());
}

#[test]
fn tool_drift_is_refused_without_burning_the_ticket() {
    let (_dir, file, now) = armed();
    let euid = crate::syscall::geteuid();
    let drift = |_: &DispatchContextV1, _: &super::super::ResearchTool| anyhow::bail!("drift");
    assert!(authorize_dispatch(&file, &argv("run"), &env(), euid, now, &drift).is_err());
    assert!(authorize_dispatch(&file, &argv("run"), &env(), euid, now, &|_, _| Ok(())).is_ok());
}

#[test]
fn arming_twice_is_refused() {
    let (_dir, file, intent, slot) = fixture();
    let now = intent.issued_unix_seconds + 1;
    let arm = |f: &std::fs::File| arm_attempt(f, &intent, &slot, spec(), (now, 60));
    arm(&file).unwrap();
    assert!(arm(&file).is_err());
}

#[test]
fn a_dispatch_without_a_launch_request_is_refused_and_burns_nothing() {
    let (_dir, file, mut intent, slot) = fixture();
    intent.boot_id = crate::telemetry::identity::read_boot_id().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    intent.issued_unix_seconds = now - 1;
    intent.deadline_unix_seconds = now + 3_600;
    arm_attempt(&file, &intent, &slot, spec(), (now, 60)).unwrap();
    let late = || {
        authorize_dispatch(&file, &argv("run"), &env(), intent.uid, now + 1, &|_, _| {
            Ok(())
        })
    };
    // A delayed invocation from an earlier attempt finds this one armed but never requested.
    assert!(late().is_err());
    mark_launch_requested(&file).unwrap();
    assert!(mark_launch_requested(&file).is_err());
    late().expect("the requested attempt's own dispatch still has its tickets");
}
