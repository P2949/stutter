use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

// TODO: DiagnosisCandidate::evidence_details and LiveDiagnosisEntry::raw_latencies are not yet
// consumed here. When implementing specific actionable recommendations, read these fields to
// produce per-IRQ/per-process evidence strings.
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    diagnosis::{Confidence, StutterCause},
    report::{self, DataQualityLevel, ReportAnalysisJson},
};

#[derive(Debug, Clone)]
pub struct AdvisorCommandInput {
    pub run: Option<PathBuf>,
    pub profiles: Option<PathBuf>,
    pub json: bool,
    pub watch_runs: bool,
    pub runs_dir: Option<PathBuf>,
    pub poll_seconds: u64,
    pub once: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorReport {
    pub schema_version: u32,
    pub run: PathBuf,
    pub data_quality: DataQualityLevel,
    pub verdict: AdvisorVerdict,
    pub recommendations: Vec<AdvisorRecommendation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdvisorVerdict {
    NoAction,
    CollectMoreData,
    TryProfileTuning,
    InvestigateNonCpuBottleneck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorRecommendation {
    pub title: String,
    pub rationale: String,
    pub confidence: Confidence,
    pub suggested_commands: Vec<String>,
    pub safety_note: String,
}

pub async fn advisor_command(input: AdvisorCommandInput) -> anyhow::Result<()> {
    if input.watch_runs {
        return watch_runs(input).await;
    }
    let run = input
        .run
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("advisor requires --run unless --watch-runs is set"))?;
    let report = build_advisor_report(run, input.profiles.as_deref())?;
    print_report(&report, input.json)?;
    Ok(())
}

pub fn build_advisor_report(run: &Path, profiles: Option<&Path>) -> anyhow::Result<AdvisorReport> {
    let analysis = report::build_report_analysis(run, 10, 5, None)?;
    Ok(build_advisor_report_from_analysis(run, profiles, &analysis))
}

pub fn build_advisor_report_from_analysis(
    run: &Path,
    profiles: Option<&Path>,
    analysis: &ReportAnalysisJson,
) -> AdvisorReport {
    let causes = causes_from_analysis(analysis);
    build_advisor_report_from_evidence(
        run,
        analysis.data_quality.level,
        &causes,
        profiles,
        analysis.session.config.hwmon || analysis.artifacts_summary.gpu_sample_count > 0,
        analysis.session.config.irq_latency || analysis.artifacts_summary.irq_event_count > 0,
        analysis.session.config.block_io || analysis.artifacts_summary.block_io_event_count > 0,
        analysis.session.config.tree_roots.first().copied(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_advisor_report_from_evidence(
    run: &Path,
    data_quality: DataQualityLevel,
    causes: &[StutterCause],
    profiles: Option<&Path>,
    has_hwmon: bool,
    has_irq: bool,
    has_block_io: bool,
    tree_pid: Option<u32>,
) -> AdvisorReport {
    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();

    if data_quality == DataQualityLevel::Low {
        recommendations.push(AdvisorRecommendation {
            title: "Collect more data".to_owned(),
            rationale:
                "Data quality is low, so advisor output is only a candidate signal and not proof."
                    .to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: vec![
                "stutter bench --duration 180 --scenario <name> --role baseline".to_owned(),
            ],
            safety_note: "Observe only; do not auto-apply tuning from this run.".to_owned(),
        });
        return AdvisorReport {
            schema_version: 1,
            run: run.to_path_buf(),
            data_quality,
            verdict: AdvisorVerdict::CollectMoreData,
            recommendations,
            warnings,
        };
    }

    let has_scheduler = causes.iter().any(|cause| {
        matches!(
            cause,
            StutterCause::CompositorSchedulerDelay | StutterCause::GameThreadSchedulerDelay
        )
    });
    let has_gpu = causes.contains(&StutterCause::GpuBoundCandidate);
    let has_irq_candidate = causes.contains(&StutterCause::IrqDelayCandidate);
    let has_block_io_candidate = causes.contains(&StutterCause::BlockIoCandidate);

    if has_gpu {
        recommendations.push(AdvisorRecommendation {
            title: "Investigate non-CPU bottleneck candidate".to_owned(),
            rationale:
                "GPU-bound evidence is a candidate, not proof; CPU affinity may not fix this."
                    .to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: if has_hwmon {
                vec!["stutter report --analysis-json <run-dir>".to_owned()]
            } else {
                vec!["stutter record --hwmon --duration 180 --run-name gpu-check".to_owned()]
            },
            safety_note: "Observe only; do not auto-apply CPU affinity for a GPU-bound candidate."
                .to_owned(),
        });
        warnings.push("CPU affinity may not help a GPU-bound candidate.".to_owned());
    }

    if has_irq_candidate {
        recommendations.push(AdvisorRecommendation {
            title: "Confirm IRQ latency candidate".to_owned(),
            rationale: "IRQ overlap is a candidate signal, not proof; collect explicit IRQ data before changing anything.".to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: if has_irq {
                vec!["stutter report --analysis-json <run-dir>".to_owned()]
            } else {
                vec!["stutter record --irq-latency --irq <IRQ> --duration 180 --run-name irq-check".to_owned()]
            },
            safety_note: "Observe only; do not change IRQ affinity yet.".to_owned(),
        });
        warnings.push("Advisor does not suggest changing IRQ affinity yet.".to_owned());
    }

    if has_block_io_candidate {
        recommendations.push(AdvisorRecommendation {
            title: "Check storage activity candidate".to_owned(),
            rationale: "Block I/O overlap is a candidate, not proof; storage pressure should be confirmed before CPU tuning.".to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: if has_block_io {
                vec!["stutter report --analysis-json <run-dir>".to_owned()]
            } else {
                vec!["stutter record --block-io --duration 180 --run-name io-check".to_owned()]
            },
            safety_note: "Observe only; do not tune CPU affinity first for a block I/O candidate.".to_owned(),
        });
    }

    if has_scheduler {
        let profiles_arg = profiles
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<profiles.toml>".to_owned());
        let pid_arg = tree_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "<PID>".to_owned());
        recommendations.push(AdvisorRecommendation {
            title: "Try profile tuning experiment".to_owned(),
            rationale: "Scheduler-delay candidates suggest a profile tuning experiment may be useful, but this is not proof of root cause.".to_owned(),
            confidence: Confidence::Medium,
            suggested_commands: vec![format!(
                "stutter tune --tree-pid {pid_arg} --profiles {profiles_arg} --runs 5 --baseline-profile baseline-online"
            )],
            safety_note: "Suggested experiment only; do not auto-apply the result.".to_owned(),
        });
        if profiles.is_none() {
            warnings.push(
                "No profiles file was provided; create one from examples/profiles before tuning."
                    .to_owned(),
            );
        }
    }

    let verdict = if has_gpu || has_irq_candidate || has_block_io_candidate {
        AdvisorVerdict::InvestigateNonCpuBottleneck
    } else if has_scheduler {
        AdvisorVerdict::TryProfileTuning
    } else {
        if recommendations.is_empty() {
            recommendations.push(AdvisorRecommendation {
                title: "Collect more comparable data".to_owned(),
                rationale:
                    "No strong candidate stood out; this is not proof that no bottleneck exists."
                        .to_owned(),
                confidence: Confidence::Low,
                suggested_commands: vec![
                    "stutter bench --duration 180 --scenario <name> --role baseline".to_owned(),
                ],
                safety_note: "Observe only; do not auto-apply tuning from this run.".to_owned(),
            });
        }
        AdvisorVerdict::CollectMoreData
    };

    AdvisorReport {
        schema_version: 1,
        run: run.to_path_buf(),
        data_quality,
        verdict,
        recommendations,
        warnings,
    }
}

pub fn render_advisor_report(report: &AdvisorReport) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter advisor");
    pushln(&mut out, "");
    pushln(&mut out, format!("Run: {}", report.run.display()));
    pushln(&mut out, format!("Data quality: {:?}", report.data_quality));
    pushln(&mut out, format!("Verdict: {:?}", report.verdict));
    pushln(&mut out, "");
    pushln(&mut out, "## Recommendations");
    pushln(&mut out, "");
    for recommendation in &report.recommendations {
        pushln(&mut out, format!("- {}", recommendation.title));
        pushln(
            &mut out,
            format!("  rationale: {}", recommendation.rationale),
        );
        pushln(
            &mut out,
            format!("  confidence: {:?}", recommendation.confidence),
        );
        pushln(
            &mut out,
            format!("  safety: {}", recommendation.safety_note),
        );
        for command in &recommendation.suggested_commands {
            pushln(&mut out, format!("  command: {command}"));
        }
    }
    if report.recommendations.is_empty() {
        pushln(&mut out, "- none");
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if report.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for warning in &report.warnings {
            pushln(&mut out, format!("- {warning}"));
        }
    }
    out
}

pub fn default_runs_dir() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("runs");
    path
}

pub fn completed_run_dirs(
    runs_dir: &Path,
    processed: &BTreeSet<PathBuf>,
) -> anyhow::Result<Vec<PathBuf>> {
    completed_run_dirs_with_min_age(runs_dir, processed, Duration::from_secs(2))
}

pub fn completed_run_dirs_with_min_age(
    runs_dir: &Path,
    processed: &BTreeSet<PathBuf>,
    min_session_age: Duration,
) -> anyhow::Result<Vec<PathBuf>> {
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut runs = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() || processed.contains(&path) {
            continue;
        }
        let session_path = path.join("session.json");
        if !session_path.exists() {
            continue;
        }
        let modified = session_path
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified.elapsed().unwrap_or(Duration::ZERO) < min_session_age {
            continue;
        }
        runs.push(path);
    }
    runs.sort();
    Ok(runs)
}

async fn watch_runs(input: AdvisorCommandInput) -> anyhow::Result<()> {
    let runs_dir = input.runs_dir.unwrap_or_else(default_runs_dir);
    let mut processed = BTreeSet::new();
    loop {
        let runs = completed_run_dirs(&runs_dir, &processed)?;
        for run in runs {
            match build_advisor_report(&run, input.profiles.as_deref()) {
                Ok(report) => {
                    print_report(&report, input.json)?;
                    processed.insert(run);
                }
                Err(err) => {
                    log::warn!(
                        "advisor_watch_run_load_failed run={} err={err:#}",
                        run.display()
                    );
                }
            }
        }
        if input.once {
            return Ok(());
        }
        sleep(Duration::from_secs(input.poll_seconds)).await;
    }
}

fn print_report(report: &AdvisorReport, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        print!("{}", render_advisor_report(report));
    }
    Ok(())
}

fn causes_from_analysis(analysis: &ReportAnalysisJson) -> Vec<StutterCause> {
    let mut causes = Vec::new();
    for cluster in analysis.cluster_analysis.clusters.iter().take(10) {
        if let Some(diagnosis) = &cluster.diagnosis {
            if let Some(primary) = &diagnosis.primary {
                causes.push(primary.cause);
            } else {
                causes.push(diagnosis.cause);
            }
            causes.extend(diagnosis.secondary_causes.iter().copied());
        }
    }
    for frame in analysis.frame_diagnoses.iter().take(10) {
        if let Some(primary) = &frame.diagnosis.primary {
            causes.push(primary.cause);
        } else {
            causes.push(frame.diagnosis.cause);
        }
    }
    causes
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-advisor-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn report_for(causes: &[StutterCause], quality: DataQualityLevel) -> AdvisorReport {
        build_advisor_report_from_evidence(
            Path::new("/tmp/run"),
            quality,
            causes,
            Some(Path::new("profiles.toml")),
            false,
            false,
            false,
            Some(42),
        )
    }

    #[test]
    fn low_data_quality_blocks_tuning_recommendation() {
        let report = report_for(
            &[StutterCause::GameThreadSchedulerDelay],
            DataQualityLevel::Low,
        );

        assert_eq!(report.verdict, AdvisorVerdict::CollectMoreData);
        assert!(
            !report
                .recommendations
                .iter()
                .any(|rec| rec.title.contains("profile tuning"))
        );
    }

    #[test]
    fn compositor_scheduler_delay_recommends_profile_tuning() {
        let report = report_for(
            &[StutterCause::CompositorSchedulerDelay],
            DataQualityLevel::High,
        );

        assert_eq!(report.verdict, AdvisorVerdict::TryProfileTuning);
        assert!(
            report.recommendations[0].suggested_commands[0].contains("stutter tune --tree-pid 42")
        );
    }

    #[test]
    fn gpu_bound_warns_cpu_affinity_may_not_help() {
        let report = report_for(&[StutterCause::GpuBoundCandidate], DataQualityLevel::High);

        assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("CPU affinity may not help"))
        );
    }

    #[test]
    fn irq_candidate_does_not_suggest_changing_irq_affinity_yet() {
        let report = report_for(&[StutterCause::IrqDelayCandidate], DataQualityLevel::High);

        assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
        assert!(
            report
                .recommendations
                .iter()
                .flat_map(|rec| rec.suggested_commands.iter())
                .all(|command| !command.contains("irq affinity"))
        );
        assert!(
            report.recommendations[0]
                .safety_note
                .contains("do not change IRQ affinity yet")
        );
    }

    #[test]
    fn unknown_result_suggests_more_data() {
        let report = report_for(&[StutterCause::Unknown], DataQualityLevel::High);

        assert_eq!(report.verdict, AdvisorVerdict::CollectMoreData);
    }

    #[test]
    fn watch_scanner_finds_completed_run_dirs() {
        let dir = temp_dir("scanner-finds");
        let run = dir.join("run-a");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("session.json"), "{}").unwrap();

        let runs = completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::ZERO).unwrap();

        assert_eq!(runs, vec![run]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn watch_scanner_ignores_dirs_without_session() {
        let dir = temp_dir("scanner-ignores");
        fs::create_dir_all(dir.join("run-a")).unwrap();

        let runs = completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::ZERO).unwrap();

        assert!(runs.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn watch_scanner_skips_recently_modified_session() {
        let dir = temp_dir("scanner-recent");
        let run = dir.join("run-a");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("session.json"), "{}").unwrap();

        let runs = completed_run_dirs_with_min_age(&dir, &BTreeSet::new(), Duration::from_secs(2))
            .unwrap();

        assert!(runs.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn watch_scanner_does_not_process_same_path_twice() {
        let dir = temp_dir("scanner-processed");
        let run = dir.join("run-a");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("session.json"), "{}").unwrap();
        let processed = BTreeSet::from([run.clone()]);

        let runs = completed_run_dirs_with_min_age(&dir, &processed, Duration::ZERO).unwrap();

        assert!(runs.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn gpu_and_scheduler_both_produce_recommendations() {
        let report = report_for(
            &[
                StutterCause::GpuBoundCandidate,
                StutterCause::GameThreadSchedulerDelay,
            ],
            DataQualityLevel::High,
        );

        assert_eq!(report.verdict, AdvisorVerdict::InvestigateNonCpuBottleneck);
        assert!(
            report
                .recommendations
                .iter()
                .any(|r| r.title.contains("non-CPU"))
        );
        assert!(
            report
                .recommendations
                .iter()
                .any(|r| r.title.contains("profile tuning"))
        );
    }
}
