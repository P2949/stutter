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
    /// The action receipts an action child recorded beside its study (absent for other studies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_receipts: Option<serde_json::Value>,
    /// What the resident runtime recorded of the game in each attempt (absent when none was kept).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_receipts: Option<serde_json::Value>,
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

    /// (attempts whose receipt shows an eligible frame source bound to the game, completed attempts)
    /// over every verified child that completed any attempt; `None` when no child did. A child with
    /// completed attempts but no receipts counts its attempts as not eligible.
    pub fn telemetry_summary(&self) -> Option<(u32, u32)> {
        let (mut eligible, mut completed) = (0u32, 0u32);
        for child in &self.children {
            let done = child
                .report
                .as_ref()
                .and_then(|report| report["attempts_completed"].as_u64())
                .and_then(|count| u32::try_from(count).ok())
                .unwrap_or(0);
            if done == 0 {
                continue;
            }
            completed = completed.saturating_add(done);
            let good = child
                .telemetry_receipts
                .as_ref()
                .and_then(serde_json::Value::as_array)
                .map_or(0, |items| {
                    items
                        .iter()
                        .filter(|item| {
                            item["frame_verdict"] == "Eligible"
                                && item["source_bound_to_game"] == true
                        })
                        .count()
                });
            eligible = eligible.saturating_add(u32::try_from(good).unwrap_or(u32::MAX).min(done));
        }
        (completed > 0).then_some((eligible, completed))
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
            StudyArms::Single => child.tools.len() == 1,
        };
    let rule_ok = match (study.stage, &child.confirmation_rule) {
        (Stage::Confirm | Stage::Holdout, Some(rule)) => rule.is_consistent(),
        (Stage::Confirm | Stage::Holdout, None) => false,
        (_, Some(_)) => false,
        (_, None) => true,
    };
    let mut rebuilt = child.clone();
    rebuilt.tools = parent.tools.clone();
    rebuilt.max_attempts = parent.max_attempts;
    rebuilt.child_study_sha256 = None;
    rebuilt.confirmation_rule = None;
    rebuilt.treatment = None;
    // The treatment is exactly the planned study's own, and only a within-source study has one.
    let treatment_ok = child.treatment == study.treatment_spec();
    bound
        && arms_ok
        && rule_ok
        && treatment_ok
        && child.max_attempts == study.min_attempts
        && rebuilt == *parent
}

pub(crate) fn verify_child(
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
    let dispositions = super::child_ledger::read_dispositions(campaign, uid, parent)?;
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
                    study_checks(&rules, &dispositions, &name, &studies, uid, parent)?;
                    selection_check(
                        &rules,
                        &dispositions,
                        &name,
                        campaign,
                        &studies,
                        uid,
                        parent,
                    )?;
                    Ok(checked)
                })
        } else {
            Err(anyhow::anyhow!("unexpected entry in the studies directory"))
        };
        let receipts = action_receipts(&studies, &name, uid);
        let telemetry = study_file(&studies, &name, uid, "telemetry-receipts.json");
        children.push(match result {
            Ok((raw, report, runtime)) => ChildVerification {
                study: name,
                reconstruction: true,
                raw_files_verified: raw,
                runtime_matched: runtime,
                problem: None,
                report: Some(report),
                action_receipts: receipts,
                telemetry_receipts: telemetry,
            },
            Err(error) => ChildVerification {
                study: name,
                reconstruction: false,
                raw_files_verified: false,
                runtime_matched: false,
                problem: Some(format!("{error:#}")),
                report: None,
                action_receipts: None,
                telemetry_receipts: None,
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
/// The receipts an action child recorded in its study directory, if any.
fn action_receipts(studies: &File, name: &str, uid: u32) -> Option<serde_json::Value> {
    study_file(studies, name, uid, "action-receipts.json")
}

/// A JSON file a study recorded in its own directory, if present and private.
fn study_file(studies: &File, name: &str, uid: u32, file: &str) -> Option<serde_json::Value> {
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), name).ok()?);
    serde_json::from_slice(&read_private(&dir, file, uid).ok()?).ok()
}

/// What the child must additionally satisfy given the funnel's own record of it:
/// - a confirmation runs under exactly the rule its pilot froze (not merely a consistent one);
/// - a qualified action study carries valid, attributable restoration receipts.
fn study_checks(
    rules: &std::collections::BTreeMap<String, super::pilot_rule::FrozenConfirmationRule>,
    dispositions: &[super::child_ledger::Disposition],
    name: &str,
    studies: &File,
    uid: u32,
    parent: &ResearchIntentV1,
) -> anyhow::Result<()> {
    use super::child_study::{Stage, StudyResult};
    let Some(decided) = dispositions
        .iter()
        .find(|study| study.study_sha256.starts_with(name))
    else {
        return Ok(());
    };
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), name)?);
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(&dir, "intent.json", uid)?)?;
    match decided.stage {
        Stage::Confirm | Stage::Holdout => {
            let planned = super::child_study::plan(parent)?
                .into_iter()
                .find(|study| study.study_sha256 == decided.study_sha256)
                .ok_or_else(|| anyhow::anyhow!("the confirmation is not in the plan"))?;
            let frozen = planned
                .depends_on
                .iter()
                .find_map(|digest| rules.get(digest))
                .ok_or_else(|| anyhow::anyhow!("no pilot froze a rule for this confirmation"))?;
            anyhow::ensure!(
                child.confirmation_rule.as_ref() == Some(frozen),
                "the confirmation does not run under its pilot's frozen rule"
            );
        }
        Stage::Action(action) if decided.result == Some(StudyResult::Qualified) => {
            let receipts = action_receipts(studies, name, uid)
                .ok_or_else(|| anyhow::anyhow!("a qualified action study has no receipts"))?;
            receipts_valid(&receipts, action, parent.uid)?;
        }
        _ => {}
    }
    Ok(())
}

/// The builds the child runs are the ones its screen selects: a pure function of the screen's sealed
/// (and reconstructed) results, the campaign's tools and, for a feature, which builds have an
/// environment control for its factor. A merely authorized but wrongly chosen build is refused.
fn selection_check(
    rules: &std::collections::BTreeMap<String, super::pilot_rule::FrozenConfirmationRule>,
    dispositions: &[super::child_ledger::Disposition],
    name: &str,
    campaign: &File,
    studies: &File,
    uid: u32,
    parent: &ResearchIntentV1,
) -> anyhow::Result<()> {
    use super::child_study::Stage;
    let Some(decided) = dispositions
        .iter()
        .find(|study| study.study_sha256.starts_with(name))
    else {
        return Ok(());
    };
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), name)?);
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(&dir, "intent.json", uid)?)?;
    if decided.stage != Stage::Screen {
        let planned = super::child_study::plan(parent)?
            .into_iter()
            .find(|study| study.study_sha256 == decided.study_sha256)
            .ok_or_else(|| anyhow::anyhow!("the study is not in the plan"))?;
        let screen = super::arm_selection::screen_of(parent, &planned)?;
        let fps = super::arm_selection::screen_fps(campaign, uid, &screen)?;
        let rule = planned
            .depends_on
            .iter()
            .find_map(|digest| rules.get(digest))
            .cloned();
        let mut expected =
            super::funnel_driver::inputs_for_study(parent, &planned, &fps, rule)?.arms;
        let mut ran: Vec<_> = child.tools.iter().map(|tool| tool.source).collect();
        expected.sort();
        ran.sort();
        anyhow::ensure!(
            expected == ran,
            "the child does not run the builds its screen selects"
        );
    }
    Ok(())
}

/// Every receipt names the permitted action, the frozen process (start time and owner), its
/// experiment, and shows the same placement before and after: the same cgroup and leader CPUs, the
/// same process identity, and every thread the original saw (that still exists) back where it was.
fn receipts_valid(
    receipts: &serde_json::Value,
    action: super::ResearchAction,
    uid: u32,
) -> anyhow::Result<()> {
    use serde_json::Value;
    let items = receipts
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| anyhow::anyhow!("the receipts are empty"))?;
    for item in items {
        anyhow::ensure!(
            item["action"] == format!("{action:?}"),
            "a receipt names another action"
        );
        anyhow::ensure!(
            item["pid"].as_u64().is_some_and(|pid| pid > 0)
                && item["start_ticks"].as_u64().is_some_and(|ticks| ticks > 0)
                && item["uid"].as_u64() == Some(u64::from(uid)),
            "a receipt does not identify the frozen process"
        );
        anyhow::ensure!(
            item["transaction"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "a receipt names no experiment"
        );
        let (before, after) = (&item["original"], &item["restored"]);
        anyhow::ensure!(
            before["cgroup"] == after["cgroup"]
                && before["cpus_allowed"] == after["cpus_allowed"]
                && before["identity"] == after["identity"]
                && !matches!(before["identity"], Value::Null),
            "a receipt shows a different placement or process after the restore"
        );
        let moved = before["tasks"].as_object().is_some_and(|tasks| {
            tasks
                .iter()
                .any(|(tid, cpus)| after["tasks"].get(tid).is_some_and(|now| now != cpus))
        });
        anyhow::ensure!(!moved, "a thread was not restored");
    }
    Ok(())
}

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
