//! Durable campaign identity and ordered phase records. This is not execution authorization.
//!
//! The resident controller must collect these facts from trusted sources, own the global
//! controller lease, and independently validate its study journal and worker restoration state.
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
};

use serde::{Deserialize, Serialize};

use super::{ResearchIntentV1, ResearchPhase, ResearchTool, SteamClientIdentity};

const MAX_FILE: u64 = 64 * 1024;
// Each attempt may traverse several phases; a 256-event ceiling would truncate the
// plan's ordinary screening and confirmation campaign long before its attempt budget.
const EVENTS_PER_ATTEMPT: usize = 16;
const EXTRA_EVENTS: usize = 256;
pub(super) const MAX_EVENTS: usize = 1_000 * EVENTS_PER_ATTEMPT + EXTRA_EVENTS;

pub(super) fn event_limit(intent: &ResearchIntentV1) -> usize {
    intent.max_attempts as usize * EVENTS_PER_ATTEMPT + EXTRA_EVENTS
}

fn is_cleanup(phase: ResearchPhase) -> bool {
    matches!(
        phase,
        ResearchPhase::Restoring
            | ResearchPhase::Completed
            | ResearchPhase::Blocked
            | ResearchPhase::Aborted
    )
}

/// Facts must be obtained independently by the resident service, not accepted from an IPC peer.
/// This data model alone does not prove the stated facts or grant any launch/mutation authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignFacts {
    pub uid: u32,
    pub boot_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_root: Option<super::StudyRootIdentity>,
    pub steam_client: SteamClientIdentity,
    pub source_sha256: String,
    pub policy_sha256: String,
    pub game_executable_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_executable_sha256: Option<String>,
    pub build_id: u64,
    pub tools: Vec<ResearchTool>,
    pub validation_override_present: bool,
    pub mutable_action_present: bool,
}

impl CampaignFacts {
    pub(super) fn validate(&self, intent: &ResearchIntentV1) -> anyhow::Result<()> {
        intent.validate()?;
        anyhow::ensure!(
            !self.validation_override_present && !self.mutable_action_present,
            "conflicting mutation or validation override"
        );
        anyhow::ensure!(
            self.uid == intent.uid
                && self.boot_id == intent.boot_id
                && self.session_id == intent.session_id
                && self.study_root == intent.study_root
                && self.steam_client == intent.steam_client
                && self.source_sha256 == intent.source_sha256
                && self.policy_sha256 == intent.policy_sha256
                && self.game_executable_sha256 == intent.game_executable_sha256
                && self.launcher_executable_sha256 == intent.launcher_executable_sha256
                && self.build_id == intent.build_id,
            "campaign context mismatch"
        );
        anyhow::ensure!(self.tools.len() <= 6, "tool inventory exceeds six sources");
        let requested: BTreeSet<_> = intent
            .tools
            .iter()
            .map(|tool| (tool.source, &tool.identity_sha256))
            .collect();
        let observed: BTreeSet<_> = self
            .tools
            .iter()
            .map(|tool| (tool.source, &tool.identity_sha256))
            .collect();
        let sources: BTreeSet<_> = self.tools.iter().map(|tool| tool.source).collect();
        let identities: BTreeSet<_> = self
            .tools
            .iter()
            .map(|tool| &tool.identity_sha256)
            .collect();
        anyhow::ensure!(
            self.tools.iter().all(|tool| {
                tool.identity_sha256.len() == 64
                    && tool
                        .identity_sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }) && sources.len() == self.tools.len()
                && identities.len() == self.tools.len()
                && requested.is_subset(&observed),
            "tool inventory mismatch"
        );
        Ok(())
    }

    fn canonical(&self) -> Self {
        let mut facts = self.clone();
        facts.tools.sort_by_key(|tool| tool.source);
        facts
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema_version: u32,
    intent: ResearchIntentV1,
    intent_sha256: String,
    facts: CampaignFacts,
    facts_sha256: String,
    peer_pid: u32,
    peer_start_ticks: u64,
    admitted_unix_seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Event {
    pub(super) sequence: u32,
    pub(super) from: ResearchPhase,
    pub(super) to: ResearchPhase,
    pub(super) previous_sha256: String,
    pub(super) at_unix_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) block_reason: Option<CampaignBlockReason>,
}

impl Event {
    pub(super) fn semantically_valid(
        &self,
        phase: ResearchPhase,
        last: u64,
        deadline: u64,
        now: Option<u64>,
    ) -> bool {
        self.from == phase
            && phase.permits(self.to)
            && (self.block_reason.is_none() || self.to == ResearchPhase::Blocked)
            && (self.block_reason != Some(CampaignBlockReason::DeadlineExpired)
                || self.at_unix_seconds >= deadline)
            && self.at_unix_seconds >= last
            && now.is_none_or(|now| self.at_unix_seconds <= now)
            && (self.at_unix_seconds < deadline || is_cleanup(self.to))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignBlockReason {
    DeadlineExpired,
    GpuHealthIncident,
}

/// A lease locks one private campaign directory; a separate resident/global lock is also needed.
pub struct CampaignLease {
    dir: File,
    uid: u32,
    phase: ResearchPhase,
    sequence: u32,
    previous_sha256: String,
    deadline_unix_seconds: u64,
    last_unix_seconds: u64,
    event_limit: usize,
    block_reason: Option<CampaignBlockReason>,
    intent_sha256: String,
    ever_launched: bool,
    study_root: Option<super::StudyRootIdentity>,
}

fn locked_dir(dir: OwnedFd, uid: u32) -> anyhow::Result<File> {
    anyhow::ensure!(uid != 0, "campaign owner must be unprivileged");
    let supplied = File::from(dir);
    let metadata = supplied.metadata()?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "untrusted campaign directory"
    );
    // A new open-file description prevents dup() from bypassing the lock.
    let dir = File::open(format!("/proc/self/fd/{}", supplied.as_raw_fd()))?;
    let reopened = dir.metadata()?;
    anyhow::ensure!(
        metadata.dev() == reopened.dev() && metadata.ino() == reopened.ino(),
        "campaign directory changed"
    );
    anyhow::ensure!(
        crate::syscall::flock_exclusive_nonblocking(dir.as_fd()),
        "campaign directory is locked"
    );
    Ok(dir)
}

fn entries(dir: &File) -> anyhow::Result<BTreeSet<String>> {
    let mut entries = BTreeSet::new();
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid campaign entry"))?;
        anyhow::ensure!(
            entries.insert(name) && entries.len() <= MAX_EVENTS + 1,
            "too many campaign entries"
        );
    }
    Ok(entries)
}

fn read_file(dir: &File, uid: u32, name: &str) -> anyhow::Result<Vec<u8>> {
    let file = crate::syscall::openat_nofollow(dir.as_fd(), name)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == uid
            && metadata.nlink() == 1
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= MAX_FILE,
        "untrusted campaign record"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_FILE, "campaign record too large");
    Ok(bytes)
}

fn write_new(dir: &File, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(bytes.len() as u64 <= MAX_FILE, "campaign record too large");
    let tmp = format!("{name}.tmp");
    let mut file = crate::syscall::create_new_at(dir.as_fd(), &tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    crate::syscall::rename_noreplace_at(dir.as_fd(), &tmp, name)?;
    dir.sync_all()?;
    Ok(())
}

impl CampaignLease {
    /// Persist an intent before acknowledging it. The peer identity must already be authenticated.
    pub fn create(
        dir: OwnedFd,
        intent: ResearchIntentV1,
        facts: CampaignFacts,
        now: u64,
        peer_pid: u32,
        peer_start_ticks: u64,
    ) -> anyhow::Result<Self> {
        facts.validate(&intent)?;
        anyhow::ensure!(
            now >= intent.issued_unix_seconds && now < intent.deadline_unix_seconds,
            "intent is not currently valid"
        );
        anyhow::ensure!(
            peer_pid != 0 && peer_start_ticks != 0,
            "unverified submitting peer"
        );
        let dir = locked_dir(dir, facts.uid)?;
        anyhow::ensure!(entries(&dir)?.is_empty(), "campaign directory is not empty");
        let facts = facts.canonical();
        let deadline_unix_seconds = intent.deadline_unix_seconds;
        let event_limit = event_limit(&intent);
        let header = Header {
            schema_version: 1,
            intent_sha256: intent.digest()?,
            facts_sha256: stutter_core::hash::sha256_hex(&serde_json::to_vec(&facts)?),
            intent,
            facts,
            peer_pid,
            peer_start_ticks,
            admitted_unix_seconds: now,
        };
        let uid = header.facts.uid;
        let intent_sha256 = header.intent_sha256.clone();
        let bytes = serde_json::to_vec(&header)?;
        write_new(&dir, "intent.json", &bytes)?;
        Ok(Self {
            dir,
            uid,
            phase: ResearchPhase::Requested,
            sequence: 0,
            previous_sha256: stutter_core::hash::sha256_hex(&bytes),
            deadline_unix_seconds,
            last_unix_seconds: now,
            event_limit,
            block_reason: None,
            intent_sha256,
            ever_launched: false,
            study_root: header.intent.study_root,
        })
    }

    /// Recover only a sealed phase boundary. The caller must additionally verify all child-study
    /// records and settle worker tokens before using the result to resume anything.
    pub fn recover(dir: OwnedFd, facts: &CampaignFacts, now: u64) -> anyhow::Result<Self> {
        Self::recover_with(dir, facts, now, false)
    }

    /// Recover a lease that was left in an in-flight phase by a crash. Only a caller that can
    /// independently retire the interrupted attempt may use this; the returned lease is still in
    /// that phase until the caller moves it through the normal restoring transition.
    pub fn recover_interrupted_launch(
        dir: OwnedFd,
        facts: &CampaignFacts,
        now: u64,
    ) -> anyhow::Result<Self> {
        Self::recover_with(dir, facts, now, true)
    }

    fn recover_with(
        dir: OwnedFd,
        facts: &CampaignFacts,
        now: u64,
        allow_interrupted: bool,
    ) -> anyhow::Result<Self> {
        let dir = locked_dir(dir, facts.uid)?;
        let mut names = entries(&dir)?;
        anyhow::ensure!(
            names.remove("intent.json"),
            "UnsafeToResume: missing intent record"
        );
        let bytes = read_file(&dir, facts.uid, "intent.json")?;
        let header: Header = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            header.schema_version == 1
                && header.peer_pid != 0
                && header.peer_start_ticks != 0
                && header.intent_sha256 == header.intent.digest()?
                && header.facts_sha256
                    == stutter_core::hash::sha256_hex(&serde_json::to_vec(&header.facts)?)
                && header.facts == facts.canonical(),
            "campaign header mismatch"
        );
        facts.validate(&header.intent)?;
        anyhow::ensure!(
            header.admitted_unix_seconds >= header.intent.issued_unix_seconds
                && header.admitted_unix_seconds < header.intent.deadline_unix_seconds
                && now >= header.admitted_unix_seconds,
            "campaign clock or admission time invalid"
        );
        let event_limit = event_limit(&header.intent);
        let mut previous = stutter_core::hash::sha256_hex(&bytes);
        let mut phase = ResearchPhase::Requested;
        let mut last_unix_seconds = header.admitted_unix_seconds;
        let mut sequence = 0_u32;
        let mut block_reason = None;
        let mut ever_launched = false;
        while !names.is_empty() {
            sequence += 1;
            anyhow::ensure!(sequence as usize <= event_limit, "too many campaign events");
            let name = format!("event-{sequence:04}.json");
            anyhow::ensure!(
                names.remove(&name),
                "UnsafeToResume: unknown or out-of-order campaign entry"
            );
            let bytes = read_file(&dir, facts.uid, &name)?;
            let event: Event = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                event.sequence == sequence
                    && event.previous_sha256 == previous
                    && event.semantically_valid(
                        phase,
                        last_unix_seconds,
                        header.intent.deadline_unix_seconds,
                        Some(now)
                    ),
                "campaign event chain mismatch"
            );
            phase = event.to;
            ever_launched |= event.to == ResearchPhase::Launching;
            block_reason = event.block_reason;
            last_unix_seconds = event.at_unix_seconds;
            previous = stutter_core::hash::sha256_hex(&bytes);
        }
        anyhow::ensure!(
            allow_interrupted
                || !matches!(
                    phase,
                    ResearchPhase::Launching
                        | ResearchPhase::Attesting
                        | ResearchPhase::Warmup
                        | ResearchPhase::Measuring
                        | ResearchPhase::Sealing
                        | ResearchPhase::EligibleAction
                        | ResearchPhase::Confirming
                        | ResearchPhase::Restoring
                ),
            "UnsafeToResume: interrupted campaign phase"
        );
        Ok(Self {
            dir,
            uid: facts.uid,
            phase,
            sequence,
            previous_sha256: previous,
            deadline_unix_seconds: header.intent.deadline_unix_seconds,
            last_unix_seconds,
            event_limit,
            block_reason,
            intent_sha256: header.intent_sha256,
            ever_launched,
            study_root: header.intent.study_root,
        })
    }

    /// Inspect an already terminal prelaunch campaign without trusting its facts for resumption.
    /// The complete chain is revalidated under the exclusive directory lock; no state is written.
    pub(crate) fn recover_terminal_prelaunch(
        dir: OwnedFd,
        owner_uid: u32,
        now: u64,
    ) -> anyhow::Result<Self> {
        let opened = File::from(dir.try_clone()?);
        let header: Header =
            serde_json::from_slice(&read_file(&opened, owner_uid, "intent.json")?)?;
        anyhow::ensure!(header.facts.uid == owner_uid, "campaign owner mismatch");
        let lease = Self::recover(dir, &header.facts, now)?;
        anyhow::ensure!(
            !lease.ever_launched
                && matches!(lease.phase, ResearchPhase::Blocked | ResearchPhase::Aborted),
            "campaign is not a settled prelaunch terminal"
        );
        Ok(lease)
    }

    pub fn phase(&self) -> ResearchPhase {
        self.phase
    }

    pub fn block_reason(&self) -> Option<CampaignBlockReason> {
        self.block_reason
    }

    pub fn intent_sha256(&self) -> &str {
        &self.intent_sha256
    }

    pub fn deadline_unix_seconds(&self) -> u64 {
        self.deadline_unix_seconds
    }

    pub fn has_started_launch(&self) -> bool {
        self.ever_launched
    }

    pub(crate) fn study_root(&self) -> Option<super::StudyRootIdentity> {
        self.study_root
    }

    fn verify_current(&self) -> anyhow::Result<()> {
        let mut names = entries(&self.dir)?;
        anyhow::ensure!(names.remove("intent.json"), "campaign header missing");
        let header = read_file(&self.dir, self.uid, "intent.json")?;
        let mut previous = stutter_core::hash::sha256_hex(&header);
        let header: Header = serde_json::from_slice(&header)?;
        let mut last_unix_seconds = header.admitted_unix_seconds;
        let mut phase = ResearchPhase::Requested;
        let mut ever_launched = false;
        let mut block_reason = None;
        for sequence in 1..=self.sequence {
            let name = format!("event-{sequence:04}.json");
            anyhow::ensure!(names.remove(&name), "campaign event missing");
            let bytes = read_file(&self.dir, self.uid, &name)?;
            let event: Event = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                event.sequence == sequence
                    && event.previous_sha256 == previous
                    && event.semantically_valid(
                        phase,
                        last_unix_seconds,
                        self.deadline_unix_seconds,
                        None
                    ),
                "campaign event chain changed"
            );
            phase = event.to;
            ever_launched |= event.to == ResearchPhase::Launching;
            block_reason = event.block_reason;
            last_unix_seconds = event.at_unix_seconds;
            previous = stutter_core::hash::sha256_hex(&bytes);
        }
        anyhow::ensure!(
            names.is_empty()
                && phase == self.phase
                && previous == self.previous_sha256
                && header.intent.deadline_unix_seconds == self.deadline_unix_seconds
                && header.intent_sha256 == self.intent_sha256
                && ever_launched == self.ever_launched
                && block_reason == self.block_reason
                && event_limit(&header.intent) == self.event_limit
                && last_unix_seconds == self.last_unix_seconds,
            "campaign record changed or contains unknown entries"
        );
        Ok(())
    }

    /// Append one phase transition. `now` must come from the controller, never the client;
    /// active-time/monotonic budgets, external work and worker actions need their own gates.
    pub fn advance(&mut self, next: ResearchPhase, now: u64) -> anyhow::Result<()> {
        self.advance_inner(next, now, None)
    }

    pub fn block_for_deadline(&mut self, now: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            now >= self.deadline_unix_seconds,
            "campaign deadline has not expired"
        );
        self.advance_inner(
            ResearchPhase::Blocked,
            now,
            Some(CampaignBlockReason::DeadlineExpired),
        )
    }

    /// Record a health stop independently of attempt journals, including before the first launch.
    pub fn block_for_health(&mut self, now: u64) -> anyhow::Result<()> {
        self.advance_inner(
            ResearchPhase::Blocked,
            now,
            Some(CampaignBlockReason::GpuHealthIncident),
        )
    }

    fn advance_inner(
        &mut self,
        next: ResearchPhase,
        now: u64,
        block_reason: Option<CampaignBlockReason>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.phase.permits(next) && (self.sequence as usize) < self.event_limit,
            "invalid campaign transition"
        );
        anyhow::ensure!(
            now >= self.last_unix_seconds && (now < self.deadline_unix_seconds || is_cleanup(next)),
            "campaign clock moved backward or deadline expired"
        );
        self.verify_current()?;
        let sequence = self.sequence + 1;
        let event = Event {
            sequence,
            from: self.phase,
            to: next,
            previous_sha256: self.previous_sha256.clone(),
            at_unix_seconds: now,
            block_reason,
        };
        let bytes = serde_json::to_vec(&event)?;
        write_new(&self.dir, &format!("event-{sequence:04}.json"), &bytes)?;
        self.phase = next;
        self.ever_launched |= next == ResearchPhase::Launching;
        self.sequence = sequence;
        self.previous_sha256 = stutter_core::hash::sha256_hex(&bytes);
        self.last_unix_seconds = now;
        self.block_reason = block_reason;
        Ok(())
    }

    pub fn owner_uid(&self) -> u32 {
        self.uid
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
