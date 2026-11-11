//! Bounded, one-use Steam launch tickets. A ticket names a source identity, never a path, command,
//! or environment. It is only useful inside the authenticated resident campaign/Steam adapter.

use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
};

use serde::{Deserialize, Serialize};

use super::{AttemptSlot, ResearchIntentV1, ResearchTool, SteamProcessIdentity};

const MAX_TICKET_BYTES: u64 = 8 * 1024;
const MAX_TICKET_LIFETIME_SECONDS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamVerb {
    Setup,
    Run,
    WaitForExitAndRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchLaunchTicketV1 {
    pub schema_version: u32,
    pub ticket_id: String,
    pub uid: u32,
    pub steam_app_id: u32,
    pub game_build_id: u64,
    pub tool: ResearchTool,
    pub steam_client_identity_sha256: String,
    pub steam_process: SteamProcessIdentity,
    pub boot_id: String,
    pub session_id: String,
    pub verb: SteamVerb,
    pub intent_sha256: String,
    pub attempt_sha256: String,
    pub issued_unix_seconds: u64,
    pub expires_unix_seconds: u64,
}

#[derive(Clone, Debug)]
pub struct LaunchTicketBinding {
    pub uid: u32,
    pub steam_app_id: u32,
    pub game_build_id: u64,
    pub tool: ResearchTool,
    pub steam_client_identity_sha256: String,
    pub steam_process: SteamProcessIdentity,
    pub boot_id: String,
    pub session_id: String,
    pub verb: SteamVerb,
    pub intent_sha256: String,
    pub attempt_sha256: String,
}

fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(DIGITS[usize::from(byte >> 4)] as char);
        value.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    value
}

fn random_ticket_id() -> anyhow::Result<String> {
    let dev = crate::syscall::open_dir_no_symlinks(std::path::Path::new("/dev"))?;
    let mut random = crate::syscall::openat_nofollow(dev.as_fd(), "urandom")?;
    let metadata = random.metadata()?;
    anyhow::ensure!(
        metadata.file_type().is_char_device() && metadata.uid() == 0,
        "untrusted random source"
    );
    let mut bytes = [0u8; 32];
    random.read_exact(&mut bytes)?;
    Ok(hex(&bytes))
}

fn private_directory(dir: OwnedFd, uid: u32) -> anyhow::Result<File> {
    anyhow::ensure!(uid != 0, "ticket owner must be unprivileged");
    let file = File::from(dir);
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "untrusted launch ticket directory"
    );
    Ok(file)
}

fn attempt_digest(slot: &AttemptSlot) -> anyhow::Result<String> {
    anyhow::ensure!(
        digest_ok(&slot.study_sha256) && digest_ok(&slot.context_sha256) && slot.index < 1_000,
        "invalid launch ticket attempt slot"
    );
    Ok(stutter_core::hash::sha256_hex(&serde_json::to_vec(slot)?))
}

fn binding(
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    verb: SteamVerb,
) -> anyhow::Result<LaunchTicketBinding> {
    intent.validate()?;
    anyhow::ensure!(intent.build_id != 0, "missing game build identity");
    let tool = intent
        .tools
        .iter()
        .find(|tool| tool.source == slot.arm)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("attempt source is absent from intent"))?;
    let steam_process = intent
        .steam_client
        .process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Steam Experimental process is not verified"))?;
    Ok(LaunchTicketBinding {
        uid: intent.uid,
        steam_app_id: intent.app_id,
        game_build_id: intent.build_id,
        tool,
        steam_client_identity_sha256: intent.steam_client.identity_sha256.clone(),
        steam_process,
        boot_id: intent.boot_id.clone(),
        session_id: intent.session_id.clone(),
        verb,
        intent_sha256: intent.digest()?,
        attempt_sha256: attempt_digest(slot)?,
    })
}

fn validate_ticket(ticket: &ResearchLaunchTicketV1, now: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        ticket.schema_version == 1,
        "unsupported launch ticket schema"
    );
    anyhow::ensure!(
        ticket.ticket_id.len() == 64 && digest_ok(&ticket.ticket_id),
        "invalid launch ticket ID"
    );
    anyhow::ensure!(
        ticket.uid != 0 && ticket.steam_app_id == 1_091_500 && ticket.game_build_id != 0,
        "invalid Steam launch target"
    );
    anyhow::ensure!(
        ticket.tool.identity_sha256.len() == 64 && digest_ok(&ticket.tool.identity_sha256),
        "invalid Proton tool identity"
    );
    anyhow::ensure!(
        digest_ok(&ticket.steam_client_identity_sha256),
        "invalid Steam client identity"
    );
    anyhow::ensure!(
        ticket.steam_process.steam_pid != 0
            && ticket.steam_process.steam_start_ticks != 0
            && ticket.steam_process.webhelper_pid != 0
            && ticket.steam_process.webhelper_start_ticks != 0
            && ticket.steam_process.steam_pid != ticket.steam_process.webhelper_pid
            && ticket.steam_process.logged_in,
        "invalid Steam process identity"
    );
    anyhow::ensure!(
        valid_component(&ticket.boot_id) && valid_component(&ticket.session_id),
        "invalid launch ticket session binding"
    );
    anyhow::ensure!(
        digest_ok(&ticket.intent_sha256) && digest_ok(&ticket.attempt_sha256),
        "invalid launch ticket campaign binding"
    );
    anyhow::ensure!(
        ticket.issued_unix_seconds <= now
            && now < ticket.expires_unix_seconds
            && ticket.expires_unix_seconds - ticket.issued_unix_seconds
                <= MAX_TICKET_LIFETIME_SECONDS,
        "expired or not-yet-valid launch ticket"
    );
    Ok(())
}

/// Persist a fresh ticket. The resident caller must already have admitted the campaign, checked
/// the safe desktop window, and bound `intent` and `slot` to its immutable schedule.
pub fn issue_launch_ticket(
    dir: OwnedFd,
    intent: &ResearchIntentV1,
    slot: &AttemptSlot,
    verb: SteamVerb,
    now: u64,
    lifetime_seconds: u64,
) -> anyhow::Result<ResearchLaunchTicketV1> {
    anyhow::ensure!(
        lifetime_seconds > 0 && lifetime_seconds <= MAX_TICKET_LIFETIME_SECONDS,
        "invalid launch ticket lifetime"
    );
    let binding = binding(intent, slot, verb)?;
    anyhow::ensure!(
        now >= intent.issued_unix_seconds && now < intent.deadline_unix_seconds,
        "intent is outside its authorized window"
    );
    let expires = now
        .checked_add(lifetime_seconds)
        .filter(|expiry| *expiry <= intent.deadline_unix_seconds)
        .ok_or_else(|| anyhow::anyhow!("ticket outlives campaign authorization"))?;
    let ticket = ResearchLaunchTicketV1 {
        schema_version: 1,
        ticket_id: random_ticket_id()?,
        uid: binding.uid,
        steam_app_id: binding.steam_app_id,
        game_build_id: binding.game_build_id,
        tool: binding.tool,
        steam_client_identity_sha256: binding.steam_client_identity_sha256,
        steam_process: binding.steam_process,
        boot_id: binding.boot_id,
        session_id: binding.session_id,
        verb: binding.verb,
        intent_sha256: binding.intent_sha256,
        attempt_sha256: binding.attempt_sha256,
        issued_unix_seconds: now,
        expires_unix_seconds: expires,
    };
    validate_ticket(&ticket, now)?;
    let dir = private_directory(dir, ticket.uid)?;
    let name = format!("ticket-{}.json", ticket.ticket_id);
    let bytes = serde_json::to_vec(&ticket)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_TICKET_BYTES,
        "launch ticket exceeds size limit"
    );
    let mut file = crate::syscall::create_new_at(dir.as_fd(), &name)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    dir.sync_all()?;
    Ok(ticket)
}

/// Atomically claim a ticket for exactly one launch. The `used-*` marker is durable before this
/// returns, so concurrent or subsequent adapter processes cannot replay it.
pub fn consume_launch_ticket(
    dir: OwnedFd,
    ticket_id: &str,
    expected: &LaunchTicketBinding,
    now: u64,
) -> anyhow::Result<ResearchLaunchTicketV1> {
    anyhow::ensure!(digest_ok(ticket_id), "invalid launch ticket ID");
    let dir = private_directory(dir, expected.uid)?;
    let name = format!("ticket-{ticket_id}.json");
    let file = crate::syscall::openat_nofollow(dir.as_fd(), &name)?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == expected.uid
            && before.nlink() == 1
            && before.mode() & 0o077 == 0
            && before.len() <= MAX_TICKET_BYTES,
        "untrusted launch ticket record"
    );
    let mut bytes = Vec::new();
    (&file).take(MAX_TICKET_BYTES + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == before.len()
            && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec(),
        "launch ticket exceeds size limit"
    );
    let ticket: ResearchLaunchTicketV1 = serde_json::from_slice(&bytes)?;
    validate_ticket(&ticket, now)?;
    anyhow::ensure!(
        ticket.ticket_id == ticket_id,
        "launch ticket filename mismatch"
    );
    anyhow::ensure!(
        ticket.uid == expected.uid
            && ticket.steam_app_id == expected.steam_app_id
            && ticket.game_build_id == expected.game_build_id
            && ticket.tool == expected.tool
            && ticket.steam_client_identity_sha256 == expected.steam_client_identity_sha256
            && ticket.steam_process == expected.steam_process
            && ticket.boot_id == expected.boot_id
            && ticket.session_id == expected.session_id
            && ticket.verb == expected.verb
            && ticket.intent_sha256 == expected.intent_sha256
            && ticket.attempt_sha256 == expected.attempt_sha256,
        "launch ticket context mismatch"
    );

    let marker = format!("used-{ticket_id}");
    let mut used = crate::syscall::create_new_at(dir.as_fd(), &marker)?;
    used.write_all(stutter_core::hash::sha256_hex(&bytes).as_bytes())?;
    used.sync_all()?;
    dir.sync_all()?;
    Ok(ticket)
}

#[cfg(test)]
#[path = "launch_ticket_tests.rs"]
mod tests;
