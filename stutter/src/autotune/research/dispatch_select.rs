//! Authenticated selection of the armed attempt a Steam dispatcher call belongs to. The wrapper does
//! not pick a campaign by directory timestamp: the dispatcher scans every campaign under the private
//! study root and accepts a call only when exactly one armed attempt holds an unused, unexpired
//! ticket for the requested Steam verb. Anything ambiguous or stale fails closed. The selection only
//! narrows the candidates; the ticket, Steam origin and tool checks that follow still authorize.

use std::{
    fs::File,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
};

use super::{
    dispatch_context::{DispatchContextV1, latest_attempt_dir},
    launch_ticket::{ResearchLaunchTicketV1, SteamVerb},
    report_store::read_private,
};

fn campaign_name(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn private_dir(dir: &File, uid: u32) -> bool {
    dir.metadata()
        .is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0)
}

/// The latest attempt directory of one dispatch directory if it holds an unused, unexpired ticket
/// for `verb`.
fn armed_in_dispatch(dispatch: &File, uid: u32, verb: SteamVerb, now: u64) -> Option<File> {
    if !private_dir(dispatch, uid) {
        return None;
    }
    let attempt = latest_attempt_dir(dispatch).ok()?;
    if !private_dir(&attempt, uid) {
        return None;
    }
    let context: DispatchContextV1 =
        serde_json::from_slice(&read_private(&attempt, "context.json", uid).ok()?).ok()?;
    let ticket_id = context.tickets.get(verb.as_steam_argument())?;
    if ticket_id.len() != 64
        || !ticket_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let ticket: ResearchLaunchTicketV1 = serde_json::from_slice(
        &read_private(&attempt, &format!("ticket-{ticket_id}.json"), uid).ok()?,
    )
    .ok()?;
    if ticket.verb != verb || ticket.expires_unix_seconds <= now {
        return None;
    }
    // A consumed ticket leaves a use marker; only an unused one is armed.
    match crate::syscall::openat_nofollow(attempt.as_fd(), &format!("used-{ticket_id}")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(attempt),
        _ => None,
    }
}

/// Every armed attempt of one campaign: its own flat schedule and each child study's own dispatch
/// directory (`studies/<digest>/dispatch`), which hold the attempts of the child-study funnel.
fn armed_attempts(study_root: &File, name: &str, uid: u32, verb: SteamVerb, now: u64) -> Vec<File> {
    let mut found = Vec::new();
    let Some(campaign) = crate::syscall::openat_dir_nofollow(study_root.as_fd(), name)
        .ok()
        .map(File::from)
        .filter(|campaign| private_dir(campaign, uid))
    else {
        return found;
    };
    let open = |parent: &File, child: &str| {
        crate::syscall::openat_dir_nofollow(parent.as_fd(), child)
            .ok()
            .map(File::from)
    };
    if let Some(attempt) = open(&campaign, "dispatch")
        .and_then(|dispatch| armed_in_dispatch(&dispatch, uid, verb, now))
    {
        found.push(attempt);
    }
    let Some(studies) = open(&campaign, "studies").filter(|dir| private_dir(dir, uid)) else {
        return found;
    };
    let Ok(entries) = std::fs::read_dir(format!("/proc/self/fd/{}", studies.as_raw_fd())) else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| campaign_name(name)) else {
            continue;
        };
        if let Some(attempt) = open(&studies, name)
            .filter(|study| private_dir(study, uid))
            .and_then(|study| open(&study, "dispatch"))
            .and_then(|dispatch| armed_in_dispatch(&dispatch, uid, verb, now))
        {
            found.push(attempt);
        }
    }
    found
}

/// Exactly one armed attempt for `verb` across all campaigns under `study_root`, or an error.
pub fn select_armed_attempt(
    study_root: &File,
    uid: u32,
    verb: SteamVerb,
    now: u64,
) -> anyhow::Result<File> {
    anyhow::ensure!(private_dir(study_root, uid), "untrusted study root");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", study_root.as_raw_fd()))? {
        let name = entry?.file_name();
        let Some(name) = name.to_str().filter(|name| campaign_name(name)) else {
            continue;
        };
        found.extend(armed_attempts(study_root, name, uid, verb, now));
    }
    match found.len() {
        1 => found
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no armed attempt")),
        0 => anyhow::bail!("no armed research attempt holds an unused ticket for this Steam verb"),
        count => anyhow::bail!(
            "{count} armed research attempts match this Steam verb; refusing to guess"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::autotune::research::{
        AttemptSlot,
        dispatch_context::{ArmSpec, arm_attempt},
    };

    fn study() -> (
        tempfile::TempDir,
        File,
        crate::autotune::research::ResearchIntentV1,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
        let mut intent = crate::autotune::research::tests::intent();
        intent.uid = crate::syscall::geteuid();
        (dir, root, intent)
    }

    /// Arm slot 0 of a campaign directory named `name`; returns its context.
    fn arm(
        root: &File,
        name: &str,
        intent: &crate::autotune::research::ResearchIntentV1,
        now: u64,
    ) -> DispatchContextV1 {
        crate::syscall::mkdir_private_at(root.as_fd(), name).unwrap();
        let campaign = File::from(crate::syscall::openat_dir_nofollow(root.as_fd(), name).unwrap());
        crate::syscall::mkdir_private_at(campaign.as_fd(), "dispatch").unwrap();
        let dispatch =
            File::from(crate::syscall::openat_dir_nofollow(campaign.as_fd(), "dispatch").unwrap());
        crate::syscall::mkdir_private_at(dispatch.as_fd(), "0000").unwrap();
        let attempt =
            File::from(crate::syscall::openat_dir_nofollow(dispatch.as_fd(), "0000").unwrap());
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        let spec = ArmSpec {
            tool_dir: "/tools/GE".into(),
            study_compat_data: "/study/compatdata/ge".into(),
            arm_env: Default::default(),
            steamapps: "/steam/steamapps".into(),
            proton_manifest: "/m.json".into(),
        };
        arm_attempt(&attempt, intent, &slot, spec, (now, 60)).unwrap()
    }

    #[test]
    fn exactly_one_armed_attempt_is_selected_and_ambiguity_or_staleness_fails_closed() {
        let (_dir, root, intent) = study();
        let now = intent.issued_unix_seconds + 1;
        let uid = intent.uid;
        let verb = SteamVerb::Run;
        // Nothing armed.
        assert!(select_armed_attempt(&root, uid, verb, now).is_err());
        let first = arm(&root, "aaaaaaaaaaaaaaaa", &intent, now);
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_ok());
        // A second armed campaign makes the choice ambiguous: refuse to guess.
        let _second = arm(&root, "bbbbbbbbbbbbbbbb", &intent, now);
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_err());
        // Consuming the first campaign's ticket leaves exactly one armed attempt.
        let first_dir = File::from(
            crate::syscall::openat_dir_nofollow(root.as_fd(), "aaaaaaaaaaaaaaaa").unwrap(),
        );
        let dispatch =
            File::from(crate::syscall::openat_dir_nofollow(first_dir.as_fd(), "dispatch").unwrap());
        let attempt =
            File::from(crate::syscall::openat_dir_nofollow(dispatch.as_fd(), "0000").unwrap());
        let id = first.tickets.get(verb.as_steam_argument()).unwrap();
        std::fs::write(
            format!(
                "/proc/self/fd/{}/used-{id}",
                std::os::fd::AsRawFd::as_raw_fd(&attempt)
            ),
            b"x",
        )
        .unwrap();
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_ok());
        // Once the tickets expire nothing is armed, whatever the directory timestamps say.
        assert!(select_armed_attempt(&root, uid, verb, now + 3600).is_err());
        // Non-campaign entries (the prefix, backups) are ignored.
        crate::syscall::mkdir_private_at(root.as_fd(), "compatdata").unwrap();
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_ok());
    }

    /// Arm slot 0 of a child study: `<campaign>/studies/<study>/dispatch/0000`.
    fn arm_child(
        root: &File,
        campaign: &str,
        study: &str,
        intent: &crate::autotune::research::ResearchIntentV1,
        now: u64,
    ) -> DispatchContextV1 {
        let open = |parent: &File, name: &str| {
            let _ = crate::syscall::mkdir_private_at(parent.as_fd(), name);
            File::from(crate::syscall::openat_dir_nofollow(parent.as_fd(), name).unwrap())
        };
        let campaign = open(root, campaign);
        let studies = open(&campaign, "studies");
        let study = open(&studies, study);
        let dispatch = open(&study, "dispatch");
        let attempt = open(&dispatch, "0000");
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        let spec = ArmSpec {
            tool_dir: "/tools/GE".into(),
            study_compat_data: "/study/compatdata/ge".into(),
            arm_env: Default::default(),
            steamapps: "/steam/steamapps".into(),
            proton_manifest: "/m.json".into(),
        };
        arm_attempt(&attempt, intent, &slot, spec, (now, 60)).unwrap()
    }

    #[test]
    fn an_armed_child_study_attempt_is_found_and_still_ambiguous_with_any_other_armed_attempt() {
        let (_dir, root, intent) = study();
        let now = intent.issued_unix_seconds + 1;
        let (uid, verb) = (intent.uid, SteamVerb::Run);
        assert!(select_armed_attempt(&root, uid, verb, now).is_err());
        // A child study's attempt, armed only under studies/<digest>/dispatch, is selectable.
        arm_child(&root, "aaaaaaaaaaaaaaaa", "1111111111111111", &intent, now);
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_ok());
        // Two armed children (or a child and a flat attempt) are never guessed between.
        arm_child(&root, "aaaaaaaaaaaaaaaa", "2222222222222222", &intent, now);
        assert!(select_armed_attempt(&root, uid, verb, now + 1).is_err());
        let (_dir2, root2, intent2) = study();
        arm_child(
            &root2,
            "aaaaaaaaaaaaaaaa",
            "1111111111111111",
            &intent2,
            now,
        );
        arm(&root2, "bbbbbbbbbbbbbbbb", &intent2, now);
        assert!(select_armed_attempt(&root2, uid, verb, now + 1).is_err());
        // Entries under studies/ that are not study digests are ignored.
        let (_dir3, root3, intent3) = study();
        arm_child(&root3, "aaaaaaaaaaaaaaaa", "notadigest", &intent3, now);
        assert!(select_armed_attempt(&root3, uid, verb, now + 1).is_err());
    }
}
