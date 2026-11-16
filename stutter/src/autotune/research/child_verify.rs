//! Independent verification of the child studies of a finished campaign. Each child lives in
//! `studies/<digest>/` with the same layout as a campaign (`intent.json`, `journal/`, `evidence/`,
//! `report.json`), so it is verified the same way: the report must equal the deterministic function
//! of its sealed journal and evidence (`report_store::load_and_verify`), and every completed
//! attempt's native files must replay against their sealed records (`raw_verify`). On top of that a
//! child's intent must be the campaign's own intent with only its arms, budget and study binding
//! changed, so a child cannot smuggle in another tool, identity or policy. Read-only.

use std::{fs::File, os::fd::AsFd, path::PathBuf};

use serde::Serialize;

use super::{
    ProtonSource, ResearchIntentV1, ResearchJournal,
    driver::build_schedule,
    raw_verify::verify_raw_evidence,
    report_store::{load_and_verify, read_private},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildVerification {
    /// The study directory (the first 16 hex of its study digest).
    pub study: String,
    /// The report is the deterministic function of the sealed records, and the intent is the
    /// campaign's own with only arms, budget and binding changed.
    pub reconstruction: bool,
    /// Every completed attempt's native files replayed against its sealed record.
    pub raw_files_verified: bool,
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildrenVerification {
    pub children: Vec<ChildVerification>,
}

impl ChildrenVerification {
    /// At least one child exists and every one reconstructed with its raw files verified.
    pub fn all_verified(&self) -> bool {
        !self.children.is_empty()
            && self
                .children
                .iter()
                .all(|child| child.reconstruction && child.raw_files_verified)
    }
}

fn hex16(name: &str) -> bool {
    name.len() == 16
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The child's intent differs from the campaign's only in its tools (a subset of the campaign's,
/// with the campaign's own identities), its attempt budget and its study binding.
pub(crate) fn derived_from(
    child: &ResearchIntentV1,
    parent: &ResearchIntentV1,
    name: &str,
) -> bool {
    let bound = child
        .child_study_sha256
        .as_deref()
        .is_some_and(|digest| digest.get(..16) == Some(name));
    let tools_ok = !child.tools.is_empty()
        && child.tools.iter().all(|tool| parent.tools.contains(tool))
        && child.max_attempts <= parent.max_attempts;
    let mut rebuilt = child.clone();
    rebuilt.tools = parent.tools.clone();
    rebuilt.max_attempts = parent.max_attempts;
    rebuilt.child_study_sha256 = None;
    bound && tools_ok && rebuilt == *parent
}

fn verify_child(
    dir: &File,
    name: &str,
    uid: u32,
    parent: &ResearchIntentV1,
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<bool> {
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(dir, "intent.json", uid)?)?;
    anyhow::ensure!(
        derived_from(&child, parent, name),
        "the child intent is not the campaign's own intent with only its arms, budget and binding changed"
    );
    // Rebuild the report from the sealed journal and evidence and compare it to the stored one.
    load_and_verify(dir, uid)?;
    let journal = ResearchJournal::open(
        crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
        uid,
        build_schedule(&child)?,
    )?;
    let evidence = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    Ok(verify_raw_evidence(&evidence, uid, &journal.verify()?, results_for)?.passed())
}

/// Verify every child study under `campaign/studies`. A campaign without that directory has no
/// children (an empty result, which is not "verified"); an unexpected entry or an unreadable
/// child is a reported problem, never skipped.
pub fn verify_children(
    campaign: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<ChildrenVerification> {
    let studies = match crate::syscall::openat_dir_nofollow(campaign.as_fd(), "studies") {
        Ok(dir) => File::from(dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ChildrenVerification {
                children: Vec::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in std::fs::read_dir(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&studies)
    ))? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let mut children = Vec::new();
    for name in names {
        let result = if hex16(&name) {
            crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)
                .map(File::from)
                .map_err(anyhow::Error::from)
                .and_then(|dir| verify_child(&dir, &name, uid, parent, results_for))
        } else {
            Err(anyhow::anyhow!("unexpected entry in the studies directory"))
        };
        children.push(match result {
            Ok(raw) => ChildVerification {
                study: name,
                reconstruction: true,
                raw_files_verified: raw,
                problem: None,
            },
            Err(error) => ChildVerification {
                study: name,
                reconstruction: false,
                raw_files_verified: false,
                problem: Some(format!("{error:#}")),
            },
        });
    }
    Ok(ChildrenVerification { children })
}
