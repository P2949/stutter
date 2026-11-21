//! Independent verification of the child studies of a finished campaign. Each child lives in
//! `studies/<digest>/` with the same layout as a campaign (`intent.json`, `journal/`, `evidence/`,
//! `report.json`), so it is verified the same way: the report must equal the deterministic function
//! of its sealed journal and evidence (`report_store::load_and_verify`), and every completed
//! attempt's native files must replay against their sealed records (`raw_verify`). On top of that a
//! child's intent must be the campaign's own intent with only its arms, budget and study binding
//! changed, so a child cannot smuggle in another tool, identity or policy. Read-only.

use std::{fs::File, os::fd::AsFd, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    ProtonSource, ResearchIntentV1, ResearchJournal,
    child_verify_rules::{
        action_receipts, attempt_count_matches, completed_attempts, missing_children,
        receipts_valid, rule_matches_pilot, study_file,
    },
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
    /// Prefix initialization cohort observed by the host; it does not establish shader-cache state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_cohort: Option<serde_json::Value>,
    /// For a holdout child: whether it reproduced its confirmation (`replication`), by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replication: Option<String>,
    /// What the resident runtime recorded of the game in each attempt (absent when none was kept).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_receipts: Option<serde_json::Value>,
    /// The journal slots of the attempts that completed (the child's sealed evidence), which each
    /// need exactly one eligible telemetry receipt.
    pub completed_attempts: Vec<u32>,
    /// The game process each completed attempt's sealed native completion names, which a telemetry
    /// receipt must be about. An attempt with no recorded process has nothing to be matched to.
    #[serde(default)]
    pub completed_processes: Vec<AttemptProcess>,
}

/// The frames a receipt counted inside the attempt's native benchmark window. A receipt whose frame
/// buckets were truncated, or that has none, counts none: it cannot show how many were in the window.
fn frames_in_window(receipt: &serde_json::Value, process: &AttemptProcess) -> u64 {
    if receipt["frame_seconds_truncated"] != false {
        return 0;
    }
    receipt["frame_seconds"]
        .as_array()
        .map(|buckets| {
            buckets
                .iter()
                .filter_map(|bucket| Some((bucket[0].as_u64()?, bucket[1].as_u64()?)))
                .filter(|(second, _)| {
                    (process.started_unix..=process.completed_unix).contains(second)
                })
                .fold(0u64, |total, (_, count)| total.saturating_add(count))
        })
        .unwrap_or(0)
}

/// The identity of the game process of one completed attempt, from its sealed native completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptProcess {
    pub slot: u32,
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
    /// The benchmark window the sealed native completion records, in whole unix seconds: the only
    /// frames that count toward a measurement are the ones inside it (not menus or loading screens).
    pub started_unix: u64,
    pub completed_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildrenVerification {
    pub children: Vec<ChildVerification>,
    /// Studies the ledger records as run (qualified, unqualified or failed) that have no directory.
    pub missing: Vec<String>,
    /// The cumulative budget replay found a problem (`campaign_budget::replay`); absent when it is clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_problem: Option<String>,
}

impl ChildrenVerification {
    /// Every planned action study qualified and its verified child carries valid, attributable
    /// restoration receipts (a child whose receipts are missing or invalid is not `reconstruction`).
    /// At least one action study must be planned: nothing is restored vacuously.
    pub fn actions_restored(&self, planned: &[super::child_ledger::Disposition]) -> bool {
        use super::child_study::{Stage, StudyResult};
        let actions: Vec<_> = planned
            .iter()
            .filter(|study| matches!(study.stage, Stage::Action(_)))
            .collect();
        !actions.is_empty()
            && actions.iter().all(|study| {
                study.result == Some(StudyResult::Qualified)
                    && self.children.iter().any(|child| {
                        study.study_sha256.starts_with(&child.study)
                            && child.reconstruction
                            && child.action_receipts.is_some()
                    })
            })
    }

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
            if child.completed_attempts.is_empty() {
                continue;
            }
            let receipts: Vec<&serde_json::Value> = child
                .telemetry_receipts
                .as_ref()
                .and_then(serde_json::Value::as_array)
                .map(|items| items.iter().collect())
                .unwrap_or_default();
            for index in &child.completed_attempts {
                completed = completed.saturating_add(1);
                // One-to-one: exactly one receipt names this attempt, and it is eligible and bound
                // to the game. Missing, duplicated or conflicting receipts do not count.
                let mine: Vec<_> = receipts
                    .iter()
                    .filter(|item| item["attempt"].as_u64() == Some(u64::from(*index)))
                    .collect();
                // The receipt must be about the very process the sealed native completion names:
                // matching only the attempt number would let another process's receipt stand in.
                let process = child
                    .completed_processes
                    .iter()
                    .find(|process| process.slot == *index);
                if let [only] = mine.as_slice()
                    && only["frame_verdict"] == "Eligible"
                    && only["source_bound_to_game"] == true
                    && process.is_some_and(|process| {
                        only["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                            && only["start_ticks"].as_u64() == Some(process.start_ticks)
                            && only["uid"].as_u64() == Some(u64::from(process.uid))
                            && frames_in_window(only, process) >= super::quality::MIN_DIRECT_FRAMES
                    })
                {
                    eligible = eligible.saturating_add(1);
                }
            }
        }
        (completed > 0).then_some((eligible, completed))
    }

    /// Whether every holdout child reproduced its confirmation: `Some(true)` when there is at least
    /// one holdout and all did, `Some(false)` when any contradicted it or reproduced nothing, and
    /// `None` when no holdout was run (that absence is its own blocker).
    pub fn holdout_replication(
        &self,
        planned: &[super::child_ledger::Disposition],
    ) -> Option<bool> {
        super::child_verify_rules::holdout_replication(&self.children, planned)
    }

    pub fn all_verified(&self) -> bool {
        !self.children.is_empty()
            && self.missing.is_empty()
            && self.budget_problem.is_none()
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
            StudyArms::Viable => child.tools.len() >= 2,
            StudyArms::Pair => child.tools.len() == 2,
            StudyArms::Single => child.tools.len() == 1,
            StudyArms::Listed => {
                let named: Vec<_> = child.tools.iter().map(|tool| tool.source).collect();
                named.len() == parent.prefix_init_arms.len()
                    && named
                        .iter()
                        .all(|arm| parent.prefix_init_arms.contains(arm))
            }
        };
    let rule_ok = match (study.stage, &child.confirmation_rule) {
        (Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs, Some(rule)) => {
            rule.is_consistent()
        }
        (Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs, None) => false,
        (_, Some(_)) => false,
        (_, None) => true,
    };
    let mut rebuilt = child.clone();
    rebuilt.tools = parent.tools.clone();
    rebuilt.max_attempts = parent.max_attempts;
    // The time limit is the study's own: the short limit, or the authorized long window.
    let attempt_seconds_ok =
        super::child_intent::attempt_seconds(parent, &study) == child.max_attempt_seconds;
    rebuilt.max_attempt_seconds = parent.max_attempt_seconds;
    rebuilt.child_study_sha256 = None;
    rebuilt.prefix_cohort = None;
    rebuilt.confirmation_rule = None;
    rebuilt.treatment = None;
    rebuilt.initialization = false;
    // The treatment is exactly the planned study's own, and only a within-source study has one.
    let treatment_ok = child.treatment == study.treatment_spec();
    bound
        && arms_ok
        && rule_ok
        && treatment_ok
        && attempt_seconds_ok
        && child.max_attempts
            == if matches!(study.stage, Stage::Pilot) {
                (child.tools.len() as u32).saturating_mul(3)
            } else {
                study.min_attempts
            }
        && child.tools
            == parent
                .tools
                .iter()
                .filter(|tool| child.tools.contains(tool))
                .cloned()
                .collect::<Vec<_>>()
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
    // A study that archived its raw files is replayed from its own copy, which no longer depends on a
    // results directory the campaign does not control; an older study is replayed from `results_for`.
    let records = journal.verify()?;
    // An initialization child measured nothing: it has no native files to replay and no runtime to match.
    let nothing_measured = child.initialization
        && super::report_store::read_evidence(&evidence, uid)?
            .values()
            .all(|item| item.native_completion.is_none());
    let raw = if super::raw_archive::exists(dir) {
        verify_raw_evidence(&evidence, uid, &records, &|source| {
            super::raw_archive::arm_root(dir, source)
        })?
    } else {
        verify_raw_evidence(&evidence, uid, &records, results_for)?
    }
    .passed()
        || nothing_measured;
    let bindings: Vec<_> = super::report_store::read_evidence(&evidence, uid)?
        .values()
        .filter_map(|item| item.native_completion.as_deref())
        .map(|native| native.runtime_binding)
        .collect();
    let runtime = nothing_measured
        || (!bindings.is_empty()
            && bindings
                .iter()
                .all(|binding| *binding == Some(super::loaded_runtime::RuntimeBinding::Matched)));
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
                budget_problem: None,
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
    let counts = super::child_ledger::read_attempt_counts(campaign, uid, parent)?;
    let mut children = Vec::new();
    for name in names {
        let result = if hex16(&name) {
            crate::syscall::openat_dir_nofollow(studies.as_fd(), &name)
                .map(File::from)
                .map_err(anyhow::Error::from)
                .and_then(|dir| {
                    verify_child(&dir, &name, uid, parent, results_for)
                        .map(|checked| (dir, checked))
                })
                .and_then(|(dir, checked)| {
                    attempt_count_matches(&counts, &dispositions, &name, &checked.1)?;
                    let disposition = dispositions
                        .iter()
                        .find(|study| study.study_sha256.starts_with(&name))
                        .ok_or_else(|| anyhow::anyhow!("child is outside the frozen plan"))?;
                    super::child_judgement::verify_qualification(
                        &dir,
                        uid,
                        parent,
                        disposition,
                        &checked.1,
                        checked.0,
                        checked.2,
                    )?;
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
        let cohort = study_file(&studies, &name, uid, "prefix-cohort.json");
        let telemetry = study_file(&studies, &name, uid, "telemetry-receipts.json");
        let completed = completed_attempts(&studies, &name, uid);
        let processes = completed_processes(&studies, &name, uid);
        // A holdout is judged against its confirmation: a sound repetition that reaches another
        // conclusion is a contradiction, not a second pass.
        let replication = super::replication::replication_of(parent, &name, &|child| {
            study_file(&studies, child, uid, "report.json")
        })
        .map(|outcome| outcome.name().to_owned());
        children.push(match result {
            Ok((raw, report, runtime)) => ChildVerification {
                study: name,
                reconstruction: true,
                raw_files_verified: raw,
                runtime_matched: runtime,
                problem: None,
                report: Some(report),
                action_receipts: receipts,
                prefix_cohort: cohort,
                replication,
                telemetry_receipts: telemetry,
                completed_attempts: completed,
                completed_processes: processes,
            },
            Err(error) => ChildVerification {
                study: name,
                reconstruction: false,
                raw_files_verified: false,
                runtime_matched: false,
                problem: Some(format!("{error:#}")),
                report: None,
                action_receipts: None,
                prefix_cohort: None,
                replication: None,
                telemetry_receipts: None,
                completed_attempts: Vec::new(),
                completed_processes: Vec::new(),
            },
        });
    }
    let present: Vec<String> = children.iter().map(|child| child.study.clone()).collect();
    Ok(ChildrenVerification {
        missing: missing_children(campaign, uid, parent, &present)?,
        budget_problem: super::campaign_budget::replay(&studies, uid, parent, &counts),
        children,
    })
}

/// The game process each completed attempt's sealed native completion names.
fn completed_processes(studies: &File, name: &str, uid: u32) -> Vec<AttemptProcess> {
    let Ok(dir) = crate::syscall::openat_dir_nofollow(studies.as_fd(), name) else {
        return Vec::new();
    };
    let dir = File::from(dir);
    let Ok(evidence) = crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence") else {
        return Vec::new();
    };
    super::report_store::read_evidence(&File::from(evidence), uid)
        .map(|items| {
            items
                .into_iter()
                .filter_map(|(slot, item)| {
                    let native = item.native_completion.as_ref()?;
                    let process = &native.process;
                    Some(AttemptProcess {
                        slot,
                        pid: process.pid,
                        start_ticks: process.start_ticks,
                        uid: process.uid,
                        started_unix: u64::try_from(native.started_unix_nanos / 1_000_000_000)
                            .unwrap_or(u64::MAX),
                        completed_unix: u64::try_from(native.completed_unix_nanos / 1_000_000_000)
                            .unwrap_or(u64::MAX),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
    // The prefix must be initialized for later studies. Its registry does not establish actual
    // shader-cache temperature, which remains a separate measurement question.
    if decided.result == Some(StudyResult::Qualified)
        && !matches!(decided.stage, Stage::Init | Stage::Screen)
    {
        anyhow::ensure!(
            child.prefix_cohort.as_deref() == Some("warm"),
            "ColdOrUnknownCohortNotComparable"
        );
    }
    match decided.stage {
        Stage::Screen | Stage::Pilot if decided.result == Some(StudyResult::Qualified) => {
            let report = load_and_verify(&dir, uid)?;
            let journal = ResearchJournal::open(
                crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal")?,
                uid,
                build_schedule(&child)?,
            )?;
            let failures: Vec<_> = journal
                .verify()?
                .into_iter()
                .filter_map(|record| record.outcome.compatibility_failure().map(str::to_owned))
                .collect();
            let evidence = File::from(crate::syscall::openat_dir_nofollow(
                dir.as_fd(),
                "evidence",
            )?);
            let evidence = super::report_store::read_evidence(&evidence, uid)?;
            let (_, runs) = super::metric_plan::run_values(
                evidence
                    .values()
                    .filter_map(|item| item.native_completion.as_deref()),
            );
            let used = u32::try_from(report.attempts_sealed)?;
            let completed = u32::try_from(report.attempts_completed)?;
            let sufficient = if decided.stage == Stage::Screen {
                super::study_executor::screen_sufficient(
                    child.max_attempts,
                    used,
                    completed,
                    &runs,
                    &failures,
                )
            } else {
                let sources: Vec<_> = child.tools.iter().map(|tool| tool.source).collect();
                super::study_executor::pilot_sufficient(
                    child.max_attempts,
                    used,
                    completed,
                    &sources,
                    &runs,
                    &failures,
                )
            };
            anyhow::ensure!(sufficient, "ScreenOrPilotViabilityUnproven");
        }
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
        Stage::MatchedCachyOs => {
            let planned = super::child_study::plan(parent)?
                .into_iter()
                .find(|study| study.study_sha256 == decided.study_sha256)
                .ok_or_else(|| anyhow::anyhow!("the matched CachyOS study is not in the plan"))?;
            let pilot = super::arm_selection::pilot_of(parent, &planned)?;
            let fps = super::arm_selection::pilot_fps_in_studies(studies, uid, &pilot)?;
            let ca_runs = fps
                .get(&super::ProtonSource::CachyOs)
                .ok_or_else(|| anyhow::anyhow!("missing CA pilot runs"))?;
            let c3_runs = fps
                .get(&super::ProtonSource::CachyOsV3)
                .ok_or_else(|| anyhow::anyhow!("missing C3 pilot runs"))?;
            let mut runs = Vec::new();
            for &val in ca_runs {
                runs.push((super::ProtonSource::CachyOs, val));
            }
            for &val in c3_runs {
                runs.push((super::ProtonSource::CachyOsV3, val));
            }
            let pilot_tail = planned
                .depends_on
                .iter()
                .find_map(|digest| rules.get(digest))
                .is_some_and(|rule| rule.tail_metric);
            let expected_rule = super::pilot_rule::rule_from_runs(&runs)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .with_tail_metric(pilot_tail);
            anyhow::ensure!(
                child.confirmation_rule.as_ref() == Some(&expected_rule),
                "the matched CachyOS study does not run under its own frozen rule"
            );
        }
        Stage::Action(action) if decided.result == Some(StudyResult::Qualified) => {
            let receipts = action_receipts(studies, name, uid)
                .ok_or_else(|| anyhow::anyhow!("a qualified action study has no receipts"))?;
            receipts_valid(&receipts, action, parent.uid)?;
            // Each receipt must be about a completed attempt the schedule treated On, and every such
            // attempt must carry one: otherwise the comparison is of the controller's policy, not of
            // an applied action, and it is not reported as a qualified action study.
            let treatments = super::treatment::treatments_of(&child)?
                .ok_or_else(|| anyhow::anyhow!("an action study has no treatment schedule"))?;
            let processes = completed_processes(studies, name, uid);
            let coverage = super::action_coverage::attribute(&receipts, &processes, &treatments)
                .map_err(|reason| anyhow::anyhow!("{reason}"))?;
            anyhow::ensure!(
                coverage.applied_on_every_treated_attempt(),
                "ActionNotAppliedOnEveryTreatedAttempt: {} of {} treated attempts",
                coverage.on_applied,
                coverage.on_completed
            );
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
    if !matches!(decided.stage, Stage::Init | Stage::Screen) {
        let planned = super::child_study::plan(parent)?
            .into_iter()
            .find(|study| study.study_sha256 == decided.study_sha256)
            .ok_or_else(|| anyhow::anyhow!("the study is not in the plan"))?;
        let fps = if decided.stage == Stage::Pilot {
            let screen = super::arm_selection::screen_of(parent, &planned)?;
            super::arm_selection::screen_fps(campaign, uid, &screen)?
        } else {
            let pilot = super::arm_selection::pilot_of(parent, &planned)?;
            super::arm_selection::pilot_fps(campaign, uid, &pilot)?
        };
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

#[cfg(test)]
#[path = "child_verify_tests.rs"]
mod tests;
