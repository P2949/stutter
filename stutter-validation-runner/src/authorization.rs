//! Authorization policy. The kernel-reported peer credentials and the root-owned authorization
//! file are the only sources of identity and permission; nothing inside a request is trusted
//! over them.
use std::{fs, os::unix::fs::MetadataExt, path::Path};

use crate::{
    model::{
        Authorization, Envelope, ErrorCode, LiveSuite, RecordProbes, Request, RunnerAction,
        SCHEMA_VERSION, TaskGrant,
    },
    sys::PeerCredentials,
};

pub type Denial = (ErrorCode, String);

fn deny(code: ErrorCode, message: impl Into<String>) -> Denial {
    (code, message.into())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Identifiers become path components, so they are restricted to a safe alphabet.
/// Boot entry ids reach `bootctl`/`grub-reboot` as plain argv (never a shell), so GRUB's nested
/// `a>b` ids are allowed, but nothing that could be read as an option or a path.
pub fn is_safe_boot_entry_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'-' | b'_' | b'.' | b'+' | b':' | b'@' | b'>' | b'=')
        })
}

pub fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !value.starts_with('.')
}

impl Authorization {
    /// Structural validation; an authorization that fails it is never used.
    pub fn validate_shape(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported authorization schema {}",
                self.schema_version
            ));
        }
        if !is_safe_identifier(&self.campaign_id) {
            return Err("campaign id is not a safe identifier".to_owned());
        }
        if !self.repository.starts_with('/') || self.repository.contains("..") {
            return Err("repository must be an absolute canonical path".to_owned());
        }
        for (name, digest) in [
            ("manifest_sha256", &self.manifest_sha256),
            ("runner_sha256", &self.runner_sha256),
            ("xtask_sha256", &self.xtask_sha256),
            ("stutter_sha256", &self.stutter_sha256),
        ] {
            if !is_hex(digest, 64) {
                return Err(format!("{name} is not a SHA-256 digest"));
            }
        }
        if self.owner_uid == 0 {
            return Err("the campaign owner must not be root".to_owned());
        }
        for grant in &self.grants {
            if !is_safe_identifier(&grant.task_id) {
                return Err("grant task id is not a safe identifier".to_owned());
            }
            if (grant.action == RunnerAction::ExecuteLiveSuite) != grant.suite.is_some() {
                return Err("only live-suite grants name a suite, and each must".to_owned());
            }
        }
        if self
            .allowed_boot_entries
            .iter()
            .any(|entry| !is_safe_boot_entry_id(entry))
            || self
                .original_boot_entry
                .as_deref()
                .is_some_and(|entry| !is_safe_boot_entry_id(entry))
        {
            return Err("boot entry id is not safe to pass to a bootloader tool".to_owned());
        }
        Ok(())
    }

    /// Loads an authorization. With `require_root_owned` the file must be owned by root and not
    /// writable by group or others, so the campaign UID cannot widen its own permissions.
    pub fn load(path: &Path, require_root_owned: bool) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("stat {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        if require_root_owned && (metadata.uid() != 0 || metadata.mode() & 0o022 != 0) {
            return Err(format!(
                "{} must be root-owned and not group/world writable",
                path.display()
            ));
        }
        let authorization: Self = serde_json::from_slice(
            &fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?,
        )
        .map_err(|error| format!("authorization is corrupt: {error}"))?;
        authorization.validate_shape()?;
        Ok(authorization)
    }

    /// Full request authorization. `online_cpus` bounds burst CPU selection.
    pub fn authorize(
        &self,
        peer: &PeerCredentials,
        envelope: &Envelope,
        online_cpus: u32,
    ) -> Result<(), Denial> {
        if peer.uid != self.owner_uid {
            return Err(deny(
                ErrorCode::UnauthorizedPeer,
                "peer uid is not the campaign owner",
            ));
        }
        if envelope.schema_version != SCHEMA_VERSION {
            return Err(deny(
                ErrorCode::SchemaMismatch,
                format!("expected schema {SCHEMA_VERSION}"),
            ));
        }
        if envelope.campaign_id != self.campaign_id {
            return Err(deny(
                ErrorCode::UnknownCampaign,
                "campaign is not authorized on this host",
            ));
        }
        if envelope.manifest_sha256 != self.manifest_sha256 {
            return Err(deny(
                ErrorCode::ManifestMismatch,
                "manifest digest differs from the authorization",
            ));
        }
        if !is_safe_identifier(&envelope.task_id) || !is_safe_identifier(&envelope.request_id) {
            return Err(deny(
                ErrorCode::InvalidArgument,
                "task and request ids must be safe identifiers",
            ));
        }
        let action = envelope.body.action();
        if !self.grants.iter().any(|grant| grant.action == action) {
            return Err(deny(
                ErrorCode::ActionNotPermitted,
                format!("{action:?} is not authorized"),
            ));
        }
        let wanted = TaskGrant {
            task_id: envelope.task_id.clone(),
            action,
            suite: envelope.body.suite(),
        };
        if !self.grants.contains(&wanted) {
            return Err(deny(
                ErrorCode::TaskNotPermitted,
                format!("task {} may not perform {action:?}", envelope.task_id),
            ));
        }
        self.check_arguments(&envelope.body, online_cpus)
    }

    fn check_arguments(&self, request: &Request, online_cpus: u32) -> Result<(), Denial> {
        let invalid = |message: &str| Err(deny(ErrorCode::InvalidArgument, message));
        match request {
            Request::ExecuteLiveSuite {
                suite,
                attempt,
                seconds,
                rounds,
                benefit,
            } => {
                if *attempt == 0 || *attempt > 9_999 {
                    return invalid("attempt must be within 1..=9999");
                }
                if seconds.is_some() && !suite.takes_seconds() {
                    return invalid("this suite takes no --seconds");
                }
                if seconds.is_some_and(|s| !(10..=60).contains(&s)) {
                    return invalid("seconds must be within 10..=60");
                }
                if rounds.is_some() && *suite != LiveSuite::Overhead {
                    return invalid("only the overhead suite takes --rounds");
                }
                if rounds.is_some_and(|r| !(6..=48).contains(&r)) {
                    return invalid("rounds must be within 6..=48");
                }
                match (suite, benefit) {
                    (LiveSuite::AffinityBenefit, Some(spec)) if spec.pair <= 9_999 => {}
                    (LiveSuite::AffinityBenefit, _) => {
                        return invalid("the benefit suite needs a benefit spec with pair <= 9999");
                    }
                    (_, Some(_)) => return invalid("only the benefit suite takes a benefit spec"),
                    (_, None) => {}
                }
                Ok(())
            }
            Request::ExecuteSchedulerBurst {
                cpu,
                burst_ms,
                period_ms,
                seconds,
            } => {
                if *cpu >= online_cpus.max(1) {
                    return invalid("cpu is not an online cpu");
                }
                if !(1..=1_000).contains(period_ms) || burst_ms >= period_ms {
                    return invalid(
                        "burst_ms must be below period_ms, which must be within 1..=1000",
                    );
                }
                if !(1..=3_600).contains(seconds) {
                    return invalid("seconds must be within 1..=3600");
                }
                Ok(())
            }
            Request::ExecuteRecording {
                attempt,
                capture,
                target_pid,
                target_starttime_ticks,
                seconds,
                probes,
            } => {
                if *attempt == 0 || *attempt > 9_999 || *capture > 9_999 {
                    return invalid("attempt must be within 1..=9999 and capture within 0..=9999");
                }
                if *target_pid <= 1 || *target_starttime_ticks == 0 {
                    return invalid("a recording needs a real target pid and start time");
                }
                if !(10..=300).contains(seconds) {
                    return invalid("seconds must be within 10..=300");
                }
                if let RecordProbes::Corpus { irq: Some(irq) } = probes
                    && *irq >= 4_096
                {
                    return invalid("irq is out of range");
                }
                Ok(())
            }
            Request::ExecuteWorkloadOverhead {
                attempt,
                seconds,
                rounds,
                work_units,
                ..
            } => {
                if *attempt == 0 || *attempt > 9_999 {
                    return invalid("attempt must be within 1..=9999");
                }
                if !(10..=60).contains(seconds) || !(6..=48).contains(rounds) {
                    return invalid("seconds must be within 10..=60 and rounds within 6..=48");
                }
                if !(1..=100_000).contains(work_units) {
                    return invalid("work_units must be within 1..=100000");
                }
                Ok(())
            }
            Request::SetNextBoot { entry_id } | Request::PrepareKernelCycle { entry_id } => {
                if !self
                    .allowed_boot_entries
                    .iter()
                    .any(|entry| entry == entry_id)
                {
                    return Err(deny(
                        ErrorCode::BootEntryNotAuthorized,
                        "boot entry is not in the authorized set",
                    ));
                }
                Ok(())
            }
            Request::Status
            | Request::StartAutotunePrivilegedWorker
            | Request::StopAutotunePrivilegedWorker
            | Request::Reboot
            | Request::ResumeCampaign
            | Request::CancelCampaign => Ok(()),
        }
    }
}
