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
    /// Every completed attempt of the child ran under its own arm's Proton installation.
    pub runtime_matched: bool,
    pub problem: Option<String>,
    /// The child's own report, present only when it reconstructed from its sealed records: the
    /// parent's flat schedule is empty in funnel mode, so the comparison lives here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildrenVerification {
    pub children: Vec<ChildVerification>,
    /// Studies the ledger records as run (qualified, unqualified or failed) that have no directory.
    pub missing: Vec<String>,
}

impl ChildrenVerification {
    /// At least one child exists and every one reconstructed with its raw files verified.
    /// At least one child exists and every one's completed attempts matched their own arm's runtime.
    pub fn runtime_qualified(&self) -> bool {
        !self.children.is_empty() && self.children.iter().all(|child| child.runtime_matched)
    }

    pub fn all_verified(&self) -> bool {
        !self.children.is_empty()
            && self.missing.is_empty()
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

/// The child's intent is exactly what `child_intent` derives for one planned study of the parent:
/// the study named by the directory (resolved by full digest from the parent's own plan), the
/// stage's own arm count and attempt budget (arms a subset of the campaign's tools), its frozen
/// confirmation rule only when the stage is a confirmation (and consistent), and nothing else
/// changed. The rule's agreement with its pilot's sealed runs is checked against the ledger.
pub(crate) fn derived_from(
    child: &ResearchIntentV1,
    parent: &ResearchIntentV1,
    name: &str,
) -> bool {
    use super::child_study::{Stage, StudyArms, plan};
    let Some(study) = plan(parent).ok().and_then(|studies| {
        studies
            .into_iter()
            .find(|s| s.study_sha256.get(..16) == Some(name))
    }) else {
        return false;
    };
    let bound = child.child_study_sha256.as_deref() == Some(study.study_sha256.as_str());
    let arms_ok = !child.tools.is_empty()
        && child.tools.iter().all(|tool| parent.tools.contains(tool))
        && match study.arms() {
            StudyArms::All => child.tools == parent.tools,
            StudyArms::Pair => child.tools.len() == 2,
        };
    let rule_ok = match (study.stage, &child.confirmation_rule) {
        (Stage::Confirm, Some(rule)) => rule.is_consistent(),
        (Stage::Confirm, None) => false,
        (_, Some(_)) => false,
        (_, None) => true,
    };
    let mut rebuilt = child.clone();
    rebuilt.tools = parent.tools.clone();
    rebuilt.max_attempts = parent.max_attempts;
    rebuilt.child_study_sha256 = None;
    rebuilt.confirmation_rule = None;
    bound && arms_ok && rule_ok && child.max_attempts == study.min_attempts && rebuilt == *parent
}

fn verify_child(
    dir: &File,
    name: &str,
    uid: u32,
    parent: &ResearchIntentV1,
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<(bool, serde_json::Value, bool)> {
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(dir, "intent.json", uid)?)?;
    anyhow::ensure!(
        derived_from(&child, parent, name),
        "the child intent is not the campaign's own intent with only its arms, budget and binding changed"
    );
    // Rebuild the report from the sealed journal and evidence and compare it to the stored one.
    let report = serde_json::to_value(load_and_verify(dir, uid)?)?;
    let journal = ResearchJournal::open(
        crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
        uid,
        build_schedule(&child)?,
    )?;
    let evidence = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    let raw = verify_raw_evidence(&evidence, uid, &journal.verify()?, results_for)?.passed();
    let bindings: Vec<_> = super::report_store::read_evidence(&evidence, uid)?
        .values()
        .filter_map(|item| item.native_completion.as_deref())
        .map(|native| native.runtime_binding)
        .collect();
    let runtime = !bindings.is_empty()
        && bindings
            .iter()
            .all(|binding| *binding == Some(super::loaded_runtime::RuntimeBinding::Matched));
    Ok((raw, report, runtime))
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
                missing: missing_children(campaign, uid, parent, &[])?,
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
    let rules = super::child_ledger::read_rules(campaign, uid, parent)?;
    let mut children = Vec::new();
    for name in names {
        let result = if hex16(&name) {
            crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)
                .map(File::from)
                .map_err(anyhow::Error::from)
                .and_then(|dir| verify_child(&dir, &name, uid, parent, results_for))
                .and_then(|checked| {
                    // A rule the ledger froze for this pilot must follow from the pilot's own runs.
                    rule_matches_pilot(&rules, &name, studies.as_fd(), uid)?;
                    Ok(checked)
                })
        } else {
            Err(anyhow::anyhow!("unexpected entry in the studies directory"))
        };
        children.push(match result {
            Ok((raw, report, runtime)) => ChildVerification {
                study: name,
                reconstruction: true,
                raw_files_verified: raw,
                runtime_matched: runtime,
                problem: None,
                report: Some(report),
            },
            Err(error) => ChildVerification {
                study: name,
                reconstruction: false,
                raw_files_verified: false,
                runtime_matched: false,
                problem: Some(format!("{error:#}")),
                report: None,
            },
        });
    }
    let present: Vec<String> = children.iter().map(|child| child.study.clone()).collect();
    Ok(ChildrenVerification {
        missing: missing_children(campaign, uid, parent, &present)?,
        children,
    })
}

/// Studies the ledger decided after running them, whose directory is absent: a subset of valid
/// children must not stand for the whole recorded funnel.
/// A pilot's recorded rule is recomputed from its sealed whole-run results; any difference is refused.
fn rule_matches_pilot(
    rules: &std::collections::BTreeMap<String, super::pilot_rule::FrozenConfirmationRule>,
    name: &str,
    studies: std::os::fd::BorrowedFd<'_>,
    uid: u32,
) -> anyhow::Result<()> {
    let Some(recorded) = rules
        .iter()
        .find(|(digest, _)| digest.get(..16) == Some(name))
        .map(|(_, rule)| rule)
    else {
        return Ok(());
    };
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies, name)?);
    let evidence = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    let runs: Vec<_> = super::report_store::read_evidence(&evidence, uid)?
        .values()
        .filter_map(|item| item.native_completion.as_deref())
        .map(|native| (native.proton_source, native.metrics.average_fps))
        .collect();
    let recomputed = super::pilot_rule::rule_from_runs(&runs)
        .map_err(|code| anyhow::anyhow!("the pilot's runs do not support a rule: {code}"))?;
    anyhow::ensure!(
        recomputed == *recorded,
        "the recorded confirmation rule is not what the pilot's sealed runs freeze"
    );
    Ok(())
}

fn missing_children(
    campaign: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    present: &[String],
) -> anyhow::Result<Vec<String>> {
    use super::child_study::StudyResult::{Failed, Qualified, Unqualified};
    Ok(
        super::child_ledger::read_dispositions(campaign, uid, parent)?
            .into_iter()
            .filter(|study| matches!(study.result, Some(Qualified | Unqualified | Failed)))
            .map(|study| study.study_sha256[..16].to_owned())
            .filter(|name| !present.contains(name))
            .collect(),
    )
}

#[cfg(test)]
#[path = "child_verify_tests.rs"]
mod tests;
