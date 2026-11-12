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
    launch_ticket::{SteamVerb, binding, consume_launch_ticket, issue_launch_ticket},
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
) -> anyhow::Result<DispatchPlan> {
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
    consume_launch_ticket(owned, ticket_id, &expected, now)?;
    Ok(plan)
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
    anyhow::ensure!(
        inventory.eligible_path(tool) == Some(context.spec.tool_dir.as_path()),
        "installed Proton differs from the frozen tool identity or location"
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
        assert_eq!(wait.program, std::path::Path::new("/tools/GE/proton"));
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
