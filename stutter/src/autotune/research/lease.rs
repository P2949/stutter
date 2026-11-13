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
const MAX_EVENTS: usize = 1_000 * EVENTS_PER_ATTEMPT + EXTRA_EVENTS;

fn event_limit(intent: &ResearchIntentV1) -> usize {
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
    fn validate(&self, intent: &ResearchIntentV1) -> anyhow::Result<()> {
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
struct Event {
    sequence: u32,
    from: ResearchPhase,
    to: ResearchPhase,
    previous_sha256: String,
    at_unix_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    block_reason: Option<CampaignBlockReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignBlockReason {
    DeadlineExpired,
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
                    && event.from == phase
                    && event.previous_sha256 == previous
                    && phase.permits(event.to)
                    && (event.block_reason.is_none() || event.to == ResearchPhase::Blocked)
                    && (event.block_reason != Some(CampaignBlockReason::DeadlineExpired)
                        || event.at_unix_seconds >= header.intent.deadline_unix_seconds)
                    && event.at_unix_seconds >= last_unix_seconds
                    && event.at_unix_seconds <= now
                    && (event.at_unix_seconds < header.intent.deadline_unix_seconds
                        || is_cleanup(event.to)),
                "campaign event chain mismatch"
            );
            phase = event.to;
            ever_launched |= event.to == ResearchPhase::Launching;
            block_reason = event.block_reason;
            last_unix_seconds = event.at_unix_seconds;
            previous = stutter_core::hash::sha256_hex(&bytes);
        }
        anyhow::ensure!(
            !matches!(
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
        for sequence in 1..=self.sequence {
            let name = format!("event-{sequence:04}.json");
            anyhow::ensure!(names.remove(&name), "campaign event missing");
            let bytes = read_file(&self.dir, self.uid, &name)?;
            let event: Event = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                event.sequence == sequence
                    && event.from == phase
                    && event.previous_sha256 == previous
                    && phase.permits(event.to)
                    && (event.block_reason.is_none() || event.to == ResearchPhase::Blocked)
                    && (event.block_reason != Some(CampaignBlockReason::DeadlineExpired)
                        || event.at_unix_seconds >= self.deadline_unix_seconds)
                    && event.at_unix_seconds >= last_unix_seconds
                    && (event.at_unix_seconds < self.deadline_unix_seconds || is_cleanup(event.to)),
                "campaign event chain changed"
            );
            phase = event.to;
            ever_launched |= event.to == ResearchPhase::Launching;
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
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn setup() -> (tempfile::TempDir, ResearchIntentV1, CampaignFacts) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut intent = super::super::tests::intent();
        intent.uid = crate::syscall::geteuid();
        let facts = CampaignFacts {
            uid: intent.uid,
            boot_id: intent.boot_id.clone(),
            session_id: intent.session_id.clone(),
            study_root: intent.study_root,
            steam_client: intent.steam_client.clone(),
            source_sha256: intent.source_sha256.clone(),
            policy_sha256: intent.policy_sha256.clone(),
            game_executable_sha256: intent.game_executable_sha256.clone(),
            launcher_executable_sha256: intent.launcher_executable_sha256.clone(),
            build_id: intent.build_id,
            tools: intent.tools.clone(),
            validation_override_present: false,
            mutable_action_present: false,
        };
        (dir, intent, facts)
    }

    fn fd(dir: &tempfile::TempDir) -> OwnedFd {
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap()
    }

    #[test]
    fn launcher_identity_is_frozen_and_absence_keeps_the_original_encoding() {
        let (_, mut intent, mut facts) = setup();
        assert!(
            serde_json::to_value(&intent)
                .unwrap()
                .get("launcher_executable_sha256")
                .is_none()
        );
        assert!(
            serde_json::to_value(&facts)
                .unwrap()
                .get("launcher_executable_sha256")
                .is_none()
        );
        intent.launcher_executable_sha256 = Some("1".repeat(64));
        facts.launcher_executable_sha256 = intent.launcher_executable_sha256.clone();
        assert!(facts.validate(&intent).is_ok());
        facts.launcher_executable_sha256 = Some("2".repeat(64));
        assert!(facts.validate(&intent).is_err());
    }

    #[test]
    fn one_lease_persists_only_ordered_transitions_and_recovers_a_boundary() {
        let (dir, intent, facts) = setup();
        let mut lease =
            CampaignLease::create(fd(&dir), intent.clone(), facts.clone(), 11, 123, 456).unwrap();
        assert!(CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).is_err());
        assert!(lease.advance(ResearchPhase::Launching, 11).is_err());
        lease.advance(ResearchPhase::Preflight, 11).unwrap();
        lease.advance(ResearchPhase::Prepared, 11).unwrap();
        drop(lease);
        let mut lease = CampaignLease::recover(fd(&dir), &facts, 12).unwrap();
        assert_eq!(lease.phase(), ResearchPhase::Prepared);
        lease.advance(ResearchPhase::AwaitSafeWindow, 12).unwrap();
        lease.advance(ResearchPhase::Restoring, 12).unwrap();
        lease.advance(ResearchPhase::Blocked, 12).unwrap();
        assert!(lease.advance(ResearchPhase::Launching, 12).is_err());
        drop(lease);
        assert_eq!(
            CampaignLease::recover(fd(&dir), &facts, 12)
                .unwrap()
                .phase(),
            ResearchPhase::Blocked
        );
    }

    #[test]
    fn changed_source_override_deadline_and_torn_journals_fail_closed() {
        let (dir, intent, facts) = setup();
        let mut override_facts = facts.clone();
        override_facts.validation_override_present = true;
        assert!(
            CampaignLease::create(fd(&dir), intent.clone(), override_facts, 11, 123, 456).is_err()
        );
        let mut busy = facts.clone();
        busy.mutable_action_present = true;
        assert!(CampaignLease::create(fd(&dir), intent.clone(), busy, 11, 123, 456).is_err());
        let mut duplicate = facts.clone();
        duplicate.tools.push(duplicate.tools[0].clone());
        assert!(CampaignLease::create(fd(&dir), intent.clone(), duplicate, 11, 123, 456).is_err());
        assert!(
            CampaignLease::create(fd(&dir), intent.clone(), facts.clone(), 3610, 123, 456).is_err()
        );
        let mut lease =
            CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
        lease.advance(ResearchPhase::Preflight, 11).unwrap();
        std::fs::write(dir.path().join("event-0001.json"), b"{}").unwrap();
        assert!(lease.advance(ResearchPhase::Prepared, 11).is_err());
        assert!(!dir.path().join("event-0002.json").exists());
        drop(lease);
        assert!(CampaignLease::recover(fd(&dir), &facts, 12).is_err());
        let (other, intent, facts) = setup();
        let lease = CampaignLease::create(fd(&other), intent, facts.clone(), 11, 123, 456).unwrap();
        drop(lease);
        let mut changed = facts.clone();
        changed.source_sha256 = "e".repeat(64);
        assert!(CampaignLease::recover(fd(&other), &changed, 12).is_err());
        let mut changed_steam = facts.clone();
        changed_steam.steam_client.steamrt64_build_id += 1;
        assert!(CampaignLease::recover(fd(&other), &changed_steam, 12).is_err());
        std::fs::write(other.path().join("event-0001.json.tmp"), b"torn").unwrap();
        assert!(CampaignLease::recover(fd(&other), &facts, 12).is_err());
    }

    #[test]
    fn an_interrupted_launch_cannot_be_replayed() {
        let (dir, intent, facts) = setup();
        let mut lease =
            CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
        for phase in [
            ResearchPhase::Preflight,
            ResearchPhase::Prepared,
            ResearchPhase::AwaitSafeWindow,
            ResearchPhase::Launching,
        ] {
            lease.advance(phase, 11).unwrap();
        }
        drop(lease);
        assert!(
            matches!(CampaignLease::recover(fd(&dir), &facts, 12), Err(error) if error.to_string().contains("UnsafeToResume"))
        );
    }

    #[test]
    fn expiry_blocks_new_work_but_allows_restoration_and_terminal_record() {
        let (dir, intent, facts) = setup();
        let mut lease =
            CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
        lease.advance(ResearchPhase::Preflight, 11).unwrap();
        lease.advance(ResearchPhase::Prepared, 12).unwrap();
        lease.advance(ResearchPhase::AwaitSafeWindow, 13).unwrap();
        drop(lease);
        let mut lease = CampaignLease::recover(fd(&dir), &facts, 3610).unwrap();
        assert!(lease.advance(ResearchPhase::Launching, 3610).is_err());
        assert!(lease.advance(ResearchPhase::Restoring, 12).is_err());
        lease.advance(ResearchPhase::Restoring, 3610).unwrap();
        lease.advance(ResearchPhase::Blocked, 3611).unwrap();
        drop(lease);
        assert_eq!(
            CampaignLease::recover(fd(&dir), &facts, 3611)
                .unwrap()
                .phase(),
            ResearchPhase::Blocked
        );
    }

    #[test]
    fn event_capacity_covers_the_authorized_attempt_budget() {
        let mut intent = super::super::tests::intent();
        intent.max_attempts = 100;
        assert!(event_limit(&intent) >= 100 * 8);
        intent.max_attempts = 1_000;
        assert_eq!(event_limit(&intent), MAX_EVENTS);
    }
}
