//! Durable, resident-authored context for the Steam-invoked dispatcher. Steam cannot carry our
//! environment, so the resident writes one private `context.json` plus one-use tickets (one per
//! permitted Steam verb). The dispatcher authenticates by directory ownership and mode, consumes
//! the verb's ticket exactly once, and only then yields a launch plan. `waitforexitandrun` and
//! `run` each have their own ticket, so a replayed or extra invocation is refused, never re-run.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use super::{
    AttemptSlot, ResearchIntentV1,
    dispatcher::{DispatchPlan, plan_dispatch},
    launch_ticket::{
        ResearchLaunchTicketV1, SteamVerb, binding, consume_launch_ticket, issue_launch_ticket,
    },
};

const CONTEXT_NAME: &str = "context.json";
const MAX_CONTEXT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchContextV1 {
    pub schema_version: u32,
    pub intent: ResearchIntentV1,
    pub slot: AttemptSlot,
    /// Steam verb argument -> ticket ID issued for it.
    pub tickets: BTreeMap<String, String>,
    pub spec: ArmSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmSpec {
    pub tool_dir: PathBuf,
    pub study_compat_data: PathBuf,
    pub arm_env: BTreeMap<String, String>,
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
}

/// Resident side: issue a ticket for each permitted verb and persist the context. Refuses to
/// overwrite an existing context so an attempt can never be silently re-armed.
pub fn arm_attempt(
    dir: &std::fs::File,
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    spec: ArmSpec,
    (now, lifetime_seconds): (u64, u64),
) -> anyhow::Result<DispatchContextV1> {
    let mut tickets = BTreeMap::new();
    for verb in [SteamVerb::WaitForExitAndRun, SteamVerb::Run] {
        let ticket = issue_launch_ticket(
            dir.try_clone()?.into(),
            intent,
            slot,
            verb,
            now,
            lifetime_seconds,
        )?;
        tickets.insert(verb.as_steam_argument().to_owned(), ticket.ticket_id);
    }
    let context = DispatchContextV1 {
        schema_version: 1,
        intent: intent.clone(),
        slot: slot.clone(),
        tickets,
        spec,
    };
    let bytes = serde_json::to_vec(&context)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_CONTEXT_BYTES,
        "dispatch context too large"
    );
    let mut file = crate::syscall::create_new_at(dir.as_fd(), CONTEXT_NAME)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    dir.sync_all()?;
    Ok(context)
}

fn read_context(dir: &std::fs::File, uid: u32) -> anyhow::Result<DispatchContextV1> {
    let file = crate::syscall::openat_nofollow(dir.as_fd(), CONTEXT_NAME)?;
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file()
            && meta.uid() == uid
            && meta.nlink() == 1
            && meta.mode() & 0o077 == 0
            && meta.len() <= MAX_CONTEXT_BYTES,
        "untrusted dispatch context"
    );
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_CONTEXT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == meta.len(),
        "dispatch context changed while reading"
    );
    let context: DispatchContextV1 = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(context.schema_version == 1, "unsupported dispatch context");
    Ok(context)
}

/// Dispatcher side. The caller must be the unprivileged intent owner; anything else is refused
/// before a ticket is touched.
pub fn authorize_dispatch(
    dir: &std::fs::File,
    argv: &[String],
    env: &BTreeMap<String, String>,
    euid: u32,
    now: u64,
    verify_tool: &dyn Fn(&DispatchContextV1, &super::ResearchTool) -> anyhow::Result<()>,
) -> anyhow::Result<(DispatchPlan, ResearchLaunchTicketV1)> {
    anyhow::ensure!(euid != 0, "dispatcher must not run as root");
    let context = read_context(dir, euid)?;
    anyhow::ensure!(
        context.intent.uid == euid,
        "dispatch context belongs to another user"
    );
    let verb = argv
        .get(1)
        .and_then(|arg| SteamVerb::from_steam_argument(arg))
        .ok_or_else(|| anyhow::anyhow!("unsupported Steam compatibility verb"))?;
    let ticket_id = context
        .tickets
        .get(verb.as_steam_argument())
        .ok_or_else(|| anyhow::anyhow!("no ticket was issued for this Steam verb"))?;
    let expected = binding(&context.intent, &context.slot, verb)?;
    // Validate the launch plan before spending the ticket so a malformed call cannot burn it.
    let plan = plan_dispatch(
        argv,
        env,
        verb,
        &context.spec.study_compat_data,
        &context.spec.tool_dir,
        &context.spec.arm_env,
    )?;
    // Re-attest the installed Proton before spending the ticket, so drift cannot burn it either.
    verify_tool(&context, &expected.tool)?;
    let owned: OwnedFd = dir.try_clone()?.into();
    let ticket = consume_launch_ticket(owned, ticket_id, &expected, now)?;
    Ok((plan, ticket))
}

/// A durable observation of the dispatcher immediately before `exec`. It is only an anchor for
/// later independent procfs and namespace checks, not proof that Steam or Cyberpunk ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatcherStartV1 {
    pub schema_version: u32,
    pub ticket_id: String,
    pub intent_sha256: String,
    pub attempt_sha256: String,
    pub verb: SteamVerb,
    pub uid: u32,
    pub boot_id: String,
    pub observed_unix_seconds: u64,
    pub pid: u32,
    pub start_ticks: u64,
    pub pid_namespace_device: u64,
    pub pid_namespace_inode: u64,
}

/// Record this process only after its one-use ticket was consumed. The process namespace is
/// recorded because a container-visible PID cannot be assumed to be the host PID.
pub fn record_dispatcher_start(
    dir: &std::fs::File,
    ticket: &ResearchLaunchTicketV1,
) -> anyhow::Result<DispatcherStartV1> {
    let uid = crate::syscall::geteuid();
    anyhow::ensure!(uid != 0 && uid == ticket.uid, "wrong dispatcher owner");
    let boot_id = crate::telemetry::identity::read_boot_id()?;
    anyhow::ensure!(boot_id == ticket.boot_id, "dispatcher boot changed");
    let observed_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    anyhow::ensure!(
        observed_unix_seconds >= ticket.issued_unix_seconds
            && observed_unix_seconds < ticket.expires_unix_seconds,
        "dispatcher ticket expired before exec"
    );
    let pid = std::process::id();
    anyhow::ensure!(pid > 1, "invalid dispatcher pid");
    let self_meta = std::fs::metadata("/proc/self")?;
    let pid_meta = std::fs::metadata(format!("/proc/{pid}"))?;
    anyhow::ensure!(
        self_meta.dev() == pid_meta.dev() && self_meta.ino() == pid_meta.ino(),
        "dispatcher procfs namespace mismatch"
    );
    let process = crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), pid)
        .ok_or_else(|| anyhow::anyhow!("dispatcher process identity unavailable"))?;
    anyhow::ensure!(
        process.uid == uid && process.start_ticks != 0,
        "invalid dispatcher identity"
    );
    let namespace = std::fs::metadata("/proc/self/ns/pid")?;
    let record = DispatcherStartV1 {
        schema_version: 1,
        ticket_id: ticket.ticket_id.clone(),
        intent_sha256: ticket.intent_sha256.clone(),
        attempt_sha256: ticket.attempt_sha256.clone(),
        verb: ticket.verb,
        uid,
        boot_id,
        observed_unix_seconds,
        pid,
        start_ticks: process.start_ticks,
        pid_namespace_device: namespace.dev(),
        pid_namespace_inode: namespace.ino(),
    };
    let name = format!("dispatcher-{}.json", ticket.verb.as_steam_argument());
    let tmp = format!("{name}.tmp");
    let mut file = crate::syscall::create_new_at(dir.as_fd(), &tmp)?;
    file.write_all(&serde_json::to_vec(&record)?)?;
    file.sync_all()?;
    crate::syscall::rename_noreplace_at(dir.as_fd(), &tmp, &name)?;
    dir.sync_all()?;
    Ok(record)
}

/// Check a dispatcher observation against its sealed ticket and live host procfs. `None` means
/// the recorded process has exited; it does not prove the game or its descendants have stopped.
/// A live match is still only an anchor, not evidence of the effective Proton or game identity.
pub fn inspect_dispatcher_start(
    dir: &std::fs::File,
    context: &DispatchContextV1,
    verb: SteamVerb,
    proc_root: &std::path::Path,
) -> anyhow::Result<Option<DispatcherStartV1>> {
    let uid = context.intent.uid;
    let meta = dir.metadata()?;
    anyhow::ensure!(
        uid != 0 && meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "untrusted dispatch directory"
    );
    let expected = binding(&context.intent, &context.slot, verb)?;
    let ticket_id = context
        .tickets
        .get(verb.as_steam_argument())
        .ok_or_else(|| anyhow::anyhow!("missing dispatch ticket"))?;
    anyhow::ensure!(
        ticket_id.len() == 64
            && ticket_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid dispatch ticket identifier"
    );
    let read = |name: &str| super::report_store::read_private(dir, name, uid);
    let ticket_bytes = read(&format!("ticket-{ticket_id}.json"))?;
    let ticket: ResearchLaunchTicketV1 = serde_json::from_slice(&ticket_bytes)?;
    let used = read(&format!("used-{ticket_id}"))?;
    anyhow::ensure!(
        used == stutter_core::hash::sha256_hex(&ticket_bytes).as_bytes(),
        "ticket use marker mismatch"
    );
    anyhow::ensure!(
        ticket.schema_version == 1
            && ticket.ticket_id == *ticket_id
            && ticket.uid == expected.uid
            && ticket.steam_app_id == expected.steam_app_id
            && ticket.game_build_id == expected.game_build_id
            && ticket.tool == expected.tool
            && ticket.steam_client_identity_sha256 == expected.steam_client_identity_sha256
            && ticket.steam_process == expected.steam_process
            && ticket.boot_id == expected.boot_id
            && ticket.session_id == expected.session_id
            && ticket.verb == verb
            && ticket.intent_sha256 == expected.intent_sha256
            && ticket.attempt_sha256 == expected.attempt_sha256
            && ticket.issued_unix_seconds >= context.intent.issued_unix_seconds
            && ticket.expires_unix_seconds > ticket.issued_unix_seconds
            && ticket.expires_unix_seconds - ticket.issued_unix_seconds <= 300
            && ticket.expires_unix_seconds <= context.intent.deadline_unix_seconds,
        "dispatcher ticket differs from the frozen context"
    );
    let record: DispatcherStartV1 = serde_json::from_slice(&read(&format!(
        "dispatcher-{}.json",
        verb.as_steam_argument()
    ))?)?;
    anyhow::ensure!(
        record.schema_version == 1
            && record.ticket_id == ticket.ticket_id
            && record.intent_sha256 == ticket.intent_sha256
            && record.attempt_sha256 == ticket.attempt_sha256
            && record.verb == verb
            && record.uid == uid
            && record.boot_id == ticket.boot_id
            && record.observed_unix_seconds >= ticket.issued_unix_seconds
            && record.observed_unix_seconds < ticket.expires_unix_seconds
            && record.pid > 1
            && record.start_ticks != 0,
        "dispatcher observation mismatch"
    );
    let read_process = |pid| crate::telemetry::proc_view::live_process(proc_root, pid);
    let Some(process) = read_process(record.pid) else {
        return match std::fs::symlink_metadata(proc_root.join(record.pid.to_string())) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            _ => Err(anyhow::anyhow!("dispatcher process is unreadable")),
        };
    };
    let observer_ns = std::fs::metadata(proc_root.join("self/ns/pid"))?;
    let process_ns = std::fs::metadata(proc_root.join(record.pid.to_string()).join("ns/pid"))?;
    anyhow::ensure!(
        observer_ns.dev() == record.pid_namespace_device
            && observer_ns.ino() == record.pid_namespace_inode
            && process_ns.dev() == record.pid_namespace_device
            && process_ns.ino() == record.pid_namespace_inode,
        "dispatcher PID namespace mismatch"
    );
    let steam = read_process(expected.steam_process.steam_pid)
        .ok_or_else(|| anyhow::anyhow!("Steam process is unavailable"))?;
    anyhow::ensure!(
        steam.uid == uid
            && steam.start_ticks == expected.steam_process.steam_start_ticks
            && process.uid == uid
            && process.start_ticks == record.start_ticks
            && process.ancestors.contains(&(
                expected.steam_process.steam_pid,
                expected.steam_process.steam_start_ticks
            ))
            && read_process(record.pid).as_ref() == Some(&process),
        "dispatcher process identity or Steam ancestry mismatch"
    );
    Ok(Some(record))
}

/// Resident side: arm a fresh numbered attempt directory under the dispatch root. The numbered
/// name makes re-arming impossible (an existing directory is an error), and the dispatcher always
/// uses the highest number, so older consumed attempts can never be selected again.
pub fn arm_attempt_in_root(
    root: &std::fs::File,
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    spec: ArmSpec,
    window: (u64, u64),
) -> anyhow::Result<DispatchContextV1> {
    let name = format!("{:04}", slot.index);
    crate::syscall::mkdir_private_at(root.as_fd(), &name)?;
    let dir = std::fs::File::from(crate::syscall::openat_dir_nofollow(root.as_fd(), &name)?);
    root.sync_all()?;
    arm_attempt(&dir, intent, slot, spec, window)
}

/// Dispatcher side: the newest attempt directory under the dispatch root.
pub fn latest_attempt_dir(root: &std::fs::File) -> anyhow::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let mut newest: Option<String> = None;
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd()))? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 dispatch entry"))?;
        anyhow::ensure!(
            name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit()),
            "unexpected dispatch root entry"
        );
        if newest.as_ref().is_none_or(|best| name > *best) {
            newest = Some(name);
        }
    }
    let name = newest.ok_or_else(|| anyhow::anyhow!("no armed attempt"))?;
    Ok(std::fs::File::from(crate::syscall::openat_dir_nofollow(
        root.as_fd(),
        &name,
    )?))
}

/// Real verifier: rediscover the installed Proton from Steam and require the exact frozen identity at
/// exactly the directory the context will exec. Any drift refuses the launch.
pub fn verify_installed_tool(
    context: &DispatchContextV1,
    tool: &super::ResearchTool,
    uid: u32,
) -> anyhow::Result<()> {
    let libraries = super::discover_steam_libraries(&context.spec.steamapps, uid)?;
    let inventory = super::inspect_proton_inventory(
        &context.spec.proton_manifest,
        &context.spec.steamapps,
        &libraries,
        uid,
    )?;
    verify_frozen_steam_client(context, inventory.steam_client.as_ref())?;
    anyhow::ensure!(
        inventory.eligible_path(tool) == Some(context.spec.tool_dir.as_path()),
        "installed Proton differs from the frozen tool identity or location"
    );
    Ok(())
}

fn verify_frozen_steam_client(
    context: &DispatchContextV1,
    observed: Option<&super::SteamClientIdentity>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        observed == Some(&context.intent.steam_client)
            && observed
                .and_then(|client| client.process.as_ref())
                .is_some(),
        "running Steam client differs from the frozen identity"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
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
        assert_eq!(record.pid, std::process::id());
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
        let now = intent.issued_unix_seconds + 1;
        let context = arm_attempt(&file, &intent, &slot, spec(), (now, 60)).unwrap();
        let (_, ticket) =
            authorize_dispatch(&file, &argv("run"), &env(), intent.uid, now + 1, &|_, _| {
                Ok(())
            })
            .unwrap();
        let proc = tempfile::tempdir().unwrap();
        for pid in [123, 200] {
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
            pid: 123,
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
        assert!(
            authorize_dispatch(&file, &argv("run"), &env(), euid, now, &|_, _| Ok(())).is_err()
        );
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
            authorize_dispatch(&file, &argv("run"), &env(), euid, now + 600, &|_, _| Ok(()))
                .is_err()
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
}

#[cfg(test)]
mod root_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn numbered_attempt_dirs_cannot_be_rearmed_and_the_newest_is_selected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = std::fs::File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
        let mut intent = super::super::tests::intent();
        intent.uid = crate::syscall::geteuid();
        let slot = |index| AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index,
            arm: intent.tools[0].source,
        };
        let spec = || ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: BTreeMap::new(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        };
        let now = intent.issued_unix_seconds + 1;
        assert!(latest_attempt_dir(&root).is_err());
        arm_attempt_in_root(&root, &intent, &slot(0), spec(), (now, 60)).unwrap();
        assert!(arm_attempt_in_root(&root, &intent, &slot(0), spec(), (now, 60)).is_err());
        arm_attempt_in_root(&root, &intent, &slot(1), spec(), (now, 60)).unwrap();
        let newest = latest_attempt_dir(&root).unwrap();
        let context = read_context(&newest, intent.uid).unwrap();
        assert_eq!(context.slot.index, 1);
    }
}
