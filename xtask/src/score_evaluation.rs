//! Empirical evaluation of `StutterScore` as a *ranking proxy within comparable
//! experiments* (never a human-perceived percentage). Evidence: score versus controlled
//! scheduler degradation level, and agreement with the direct runnable-latency and, when the
//! recording has a frame stream, frame-time metrics. Weights are retained unless the data
//! shows the proxy is not monotonic; semantic objectives keep their own primaries regardless
//! of any score change.
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter::api::{
    autotune::runnable_signals_for_target,
    recorder::{FrameEvent, IntervalRecord, SessionFile},
    scorer::{
        frame_time_percentile_ms, score_from_interval_records,
        score_from_interval_records_and_frames,
    },
};

use crate::score_target::{self, IdentitySource, TargetIdentity};

/// Frame thresholds shared with the production score (`score_from_interval_records_and_frames`).
const FRAME_16_6_MS: f64 = 16.6;
const FRAME_33_3_MS: f64 = 33.3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelObservation {
    pub level_ms: f64,
    /// Frame-inclusive when this level has frames, otherwise runnable-only.
    pub score_total: f64,
    /// Always the runnable-only production score, so levels stay comparable when frame data is
    /// only available for some of them.
    pub runnable_score_total: f64,
    pub over_1ms: f64,
    pub over_2ms: f64,
    pub over_5ms: f64,
    pub runnable_p99_ns: Option<f64>,
    pub runnable_over_5ms_rate: Option<f64>,
    /// Frame metrics are `None` (never zero) when the recording has no usable frames.
    pub frame_p99_ms: Option<f64>,
    pub frame_p99_9_ms: Option<f64>,
    pub frame_over_16_6_rate: Option<f64>,
    pub frame_over_33_3_rate: Option<f64>,
    pub frame_samples: Option<usize>,
    /// The resolved target; `None` for synthetic observations.
    pub identity: Option<TargetIdentity>,
}

impl LevelObservation {
    fn has_frames(&self) -> bool {
        self.frame_samples.is_some_and(|samples| samples > 0)
    }
}

/// Production score and direct metrics for one controlled-degradation recording.
pub fn observe(
    level_ms: f64,
    records: &[IntervalRecord],
    frames: &[FrameEvent],
    pid: u32,
    ticks: u64,
) -> LevelObservation {
    let target = records
        .iter()
        .filter(|r| r.process_pid == Some(pid))
        .cloned()
        .collect::<Vec<_>>();
    let runnable_score = score_from_interval_records(&target);
    let signals = runnable_signals_for_target(records, pid, ticks);
    let frames = frames
        .iter()
        .filter(|frame| frame.frametime_ms.is_finite())
        .cloned()
        .collect::<Vec<_>>();
    let rate = |threshold: f64| {
        frames.iter().filter(|f| f.frametime_ms > threshold).count() as f64 / frames.len() as f64
    };
    let has_frames = !frames.is_empty();
    let score_total = if has_frames {
        score_from_interval_records_and_frames(&target, &frames).total
    } else {
        runnable_score.total
    };
    LevelObservation {
        level_ms,
        score_total: score_total as f64,
        runnable_score_total: runnable_score.total as f64,
        over_1ms: runnable_score.over_1ms as f64,
        over_2ms: runnable_score.over_2ms as f64,
        over_5ms: runnable_score.over_5ms as f64,
        runnable_p99_ns: signals.runnable_latency_p99_ns.map(|v| v as f64),
        runnable_over_5ms_rate: signals
            .runnable_over_5ms
            .zip(signals.runnable_samples.filter(|s| *s > 0))
            .map(|(over, samples)| over as f64 / samples as f64),
        frame_p99_ms: frame_time_percentile_ms(&frames, 0.99),
        frame_p99_9_ms: frame_time_percentile_ms(&frames, 0.999),
        frame_over_16_6_rate: has_frames.then(|| rate(FRAME_16_6_MS)),
        frame_over_33_3_rate: has_frames.then(|| rate(FRAME_33_3_MS)),
        frame_samples: Some(frames.len()),
        identity: None,
    }
}

/// Average ranks (ties share the mean rank).
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start;
        while end + 1 < order.len() && values[order[end + 1]] == values[order[start]] {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0 + 1.0;
        for index in &order[start..=end] {
            ranks[*index] = rank;
        }
        start = end + 1;
    }
    ranks
}

/// Spearman rank correlation; `None` when either series is constant or too short.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 3 {
        return None;
    }
    let (rx, ry) = (ranks(x), ranks(y));
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (mx, my) = (mean(&rx), mean(&ry));
    let cov = rx
        .iter()
        .zip(&ry)
        .map(|(a, b)| (a - mx) * (b - my))
        .sum::<f64>();
    let var = |v: &[f64], m: f64| v.iter().map(|a| (a - m).powi(2)).sum::<f64>();
    let denom = (var(&rx, mx) * var(&ry, my)).sqrt();
    (denom > 0.0).then(|| cov / denom)
}

pub fn evaluate(
    mut observations: Vec<LevelObservation>,
    require_frames: bool,
) -> anyhow::Result<Value> {
    ensure!(
        observations.len() >= 4,
        "need at least four degradation levels, have {}",
        observations.len()
    );
    observations.sort_by(|a, b| a.level_ms.total_cmp(&b.level_ms));
    ensure!(
        observations[0].level_ms == 0.0,
        "the zero-degradation control level is required"
    );
    let legacy = observations
        .iter()
        .filter(|o| {
            o.identity
                .is_some_and(|i| i.source == IdentitySource::LegacySampleHeuristic)
        })
        .map(|o| o.level_ms)
        .collect::<Vec<_>>();
    ensure!(
        !require_frames || legacy.is_empty(),
        "--require-frames cannot use the legacy sample-heuristic target (levels {legacy:?} ms)"
    );
    let without_frames = observations
        .iter()
        .filter(|o| !o.has_frames())
        .map(|o| o.level_ms)
        .collect::<Vec<_>>();
    ensure!(
        !require_frames || without_frames.is_empty(),
        "--require-frames: no usable frame data at degradation levels {without_frames:?} ms"
    );
    // Frame-inclusive scoring only when every level has frames; otherwise every level is
    // scored runnable-only so the series stays comparable. Absent frames are never zeros.
    let frame_basis = without_frames.is_empty();
    let levels = observations.iter().map(|o| o.level_ms).collect::<Vec<_>>();
    let scores = observations
        .iter()
        .map(|o| {
            if frame_basis {
                o.score_total
            } else {
                o.runnable_score_total
            }
        })
        .collect::<Vec<_>>();
    let inversions = scores.windows(2).filter(|w| w[1] < w[0]).count();
    let column = |select: fn(&LevelObservation) -> Option<f64>| -> Option<Vec<f64>> {
        observations.iter().map(select).collect()
    };
    let against = |name: &str, series: Option<Vec<f64>>| {
        (name.to_owned(), series.and_then(|s| spearman(&scores, &s)))
    };
    let correlations = [
        against("runnable_p99", column(|o| o.runnable_p99_ns)),
        against(
            "runnable_over_5ms_rate",
            column(|o| o.runnable_over_5ms_rate),
        ),
        against(
            "sample_over_1ms",
            Some(observations.iter().map(|o| o.over_1ms).collect()),
        ),
        against("frame_p99", column(|o| o.frame_p99_ms)),
        against("frame_p99_9", column(|o| o.frame_p99_9_ms)),
        against("frame_over_16_6_rate", column(|o| o.frame_over_16_6_rate)),
        against("frame_over_33_3_rate", column(|o| o.frame_over_33_3_rate)),
    ];
    let level_rho = spearman(&levels, &scores);
    let monotonic = inversions == 0 && level_rho.is_some_and(|rho| rho >= 0.9);
    let frame_note = if frame_basis {
        "frame metrics present at every level; score includes the frame terms"
    } else if without_frames.len() == observations.len() {
        "no frame stream in these recordings: runnable-only evaluation; frame metrics unavailable (not zero) and the frame-aware evaluation (#33) is NOT closed"
    } else {
        "frame data at only some levels: every level is scored runnable-only; frame correlations unavailable"
    };
    Ok(json!({
        "schema_version": 2,
        "interpretation": "ranking proxy within comparable experiments; not a human-perceived percentage",
        "score_basis": if frame_basis { "runnable_plus_frames" } else { "runnable_only" },
        "levels_ms": levels,
        "score_total": scores,
        "targets": observations.iter().map(|o| match o.identity {
            Some(target) => json!({"level_ms":o.level_ms,"target_pid":target.pid,
                "target_starttime_ticks":target.starttime_ticks,
                "target_identity_source":target.source.as_str()}),
            None => json!({"level_ms":o.level_ms,"target_identity_source":null}),
        }).collect::<Vec<_>>(),
        "frame_samples": observations.iter().map(|o| o.frame_samples).collect::<Vec<_>>(),
        "adjacent_inversions": inversions,
        "spearman_score_vs_level": level_rho,
        "spearman_score_vs": correlations.iter().map(|(n, v)| (n.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "monotonic_in_degradation": monotonic,
        "weights": "retained (100 x >5 ms, 20 x >2 ms, 1 x >1 ms; frame terms unchanged)",
        "weights_decision": if monotonic { "retain: no empirical case for change" } else { "review: score is not monotonic in controlled degradation" },
        "primaries_unchanged": ["GameRunnableLatency: runnable latency is primary", "CompileThroughputWithForegroundProtection: fixed-work elapsed time is primary"],
        "frame_evidence": frame_note,
    }))
}

pub fn run_from_recordings(
    levels: &[(f64, std::path::PathBuf)],
    require_frames: bool,
) -> anyhow::Result<Value> {
    let mut observations = Vec::new();
    for (level, dir) in levels {
        let records = std::fs::read_to_string(dir.join("interval.json"))?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<IntervalRecord>)
            .collect::<Result<Vec<_>, _>>()?;
        let session: SessionFile =
            serde_json::from_slice(&std::fs::read(dir.join("session.json"))?)?;
        // `--require-frames` implies strict target identity: the closure evidence must be
        // bound to the recorded target, never to whichever process had the most samples.
        let target = score_target::resolve(
            &session.config.tree_roots,
            &session.config.manual_pids,
            &session.tasks,
            &records,
            require_frames,
        )
        .with_context(|| format!("level {level} ms ({})", dir.display()))?;
        let frames = load_frames(dir)?;
        let mut observation = observe(
            *level,
            &records,
            &frames,
            target.pid,
            target.starttime_ticks,
        );
        observation.identity = Some(target);
        observations.push(observation);
    }
    evaluate(observations, require_frames)
}

/// Frame stream of a recording (NDJSON): the canonical `frame_correlation.json`, then the
/// legacy `frame_events.json` alias. A missing file is an empty stream, never an error.
fn load_frames(dir: &std::path::Path) -> anyhow::Result<Vec<FrameEvent>> {
    for name in ["frame_correlation.json", "frame_events.json"] {
        let path = dir.join(name);
        if path.is_file() {
            return std::fs::read_to_string(&path)?
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    serde_json::from_str::<FrameEvent>(line)
                        .with_context(|| format!("parse {}", path.display()))
                })
                .collect();
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(level: f64, score: f64, p99: f64) -> LevelObservation {
        LevelObservation {
            level_ms: level,
            score_total: score,
            runnable_score_total: score,
            over_1ms: score,
            runnable_p99_ns: Some(p99),
            runnable_over_5ms_rate: Some(score / 1e4),
            ..LevelObservation::default()
        }
    }

    #[test]
    fn spearman_handles_ties_constants_and_direction() {
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]), Some(1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]), Some(-1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[5.0, 5.0, 5.0]), None);
        assert!(spearman(&[1.0, 2.0], &[1.0, 2.0]).is_none());
        let tied = spearman(&[1.0, 2.0, 3.0, 4.0], &[1.0, 1.0, 2.0, 3.0]).unwrap();
        assert!(tied > 0.9 && tied < 1.0);
    }

    #[test]
    fn monotonic_scores_retain_weights_and_inversions_force_review() {
        let good = (0..6)
            .map(|i| {
                obs(
                    [0.0, 1.0, 2.0, 4.0, 8.0, 16.0][i],
                    i as f64 * 10.0,
                    1e6 * (i + 1) as f64,
                )
            })
            .collect();
        let report = evaluate(good, false).unwrap();
        assert_eq!(report["monotonic_in_degradation"], true);
        assert!(
            report["weights_decision"]
                .as_str()
                .unwrap()
                .starts_with("retain")
        );
        let bad = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0]
            .iter()
            .zip([0.0, 30.0, 20.0, 40.0, 50.0, 60.0])
            .map(|(l, s)| obs(*l, s, 1e6))
            .collect();
        let report = evaluate(bad, false).unwrap();
        assert_eq!(report["monotonic_in_degradation"], false);
        assert_eq!(report["adjacent_inversions"], 1);
        assert!(
            report["weights_decision"]
                .as_str()
                .unwrap()
                .starts_with("review")
        );
    }

    #[test]
    fn requires_four_levels_including_the_zero_control() {
        assert!(evaluate(vec![obs(0.0, 0.0, 1.0), obs(1.0, 1.0, 1.0)], false).is_err());
        assert!(
            evaluate(
                (1..=5).map(|i| obs(i as f64, i as f64, 1.0)).collect(),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn semantic_objective_primaries_are_not_the_generic_score() {
        use stutter::api::autotune::ObjectiveKind;
        let runnable = ObjectiveKind::GameRunnableLatency
            .descriptor()
            .primary_metric;
        assert!(runnable.to_lowercase().contains("runnable"), "{runnable}");
        let compile = ObjectiveKind::CompileThroughputWithForegroundProtection
            .descriptor()
            .primary_metric;
        assert!(
            compile.to_lowercase().contains("elapsed") || compile.to_lowercase().contains("fixed"),
            "{compile}"
        );
    }

    const LEVELS: [f64; 6] = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0];

    fn frame_events(times_ms: &[f64]) -> Vec<FrameEvent> {
        times_ms
            .iter()
            .enumerate()
            .map(|(i, frametime_ms)| FrameEvent {
                elapsed_ms: i as u64 * 17,
                frametime_ms: *frametime_ms,
            })
            .collect()
    }

    fn with_frames(level: f64, score: f64, p99_9: f64, frames: usize) -> LevelObservation {
        LevelObservation {
            frame_p99_ms: Some(p99_9),
            frame_p99_9_ms: Some(p99_9),
            frame_over_16_6_rate: Some(score / 100.0),
            frame_over_33_3_rate: Some(score / 200.0),
            frame_samples: Some(frames),
            ..obs(level, score, 1e6 * (score + 1.0))
        }
    }

    #[test]
    fn absent_frames_stay_unavailable_and_keep_the_runnable_only_path() {
        let report = evaluate(
            LEVELS
                .iter()
                .enumerate()
                .map(|(i, l)| obs(*l, i as f64 * 10.0, 1e6 * (i + 1) as f64))
                .collect(),
            false,
        )
        .unwrap();
        assert_eq!(report["score_basis"], "runnable_only");
        assert_eq!(report["monotonic_in_degradation"], true);
        // Unavailable is null, never a fabricated zero correlation.
        assert!(report["spearman_score_vs"]["frame_p99_9"].is_null());
        assert!(
            report["frame_evidence"]
                .as_str()
                .unwrap()
                .contains("NOT closed")
        );
        // An observation without frames reports `None`, not zeros.
        let observed = observe(0.0, &[], &[], 1, 1);
        assert_eq!(
            (
                observed.frame_p99_ms,
                observed.frame_p99_9_ms,
                observed.frame_samples
            ),
            (None, None, Some(0))
        );
        assert_eq!(observed.frame_over_16_6_rate, None);
    }

    #[test]
    fn frame_percentiles_and_rates_use_nearest_rank_and_strict_thresholds() {
        // 1000 frames: 990 at 10 ms, 8 at 20 ms, 1 at 40 ms, 1 at 80 ms.
        let mut times = vec![10.0; 990];
        times.extend([20.0; 8]);
        times.extend([40.0, 80.0]);
        let observed = observe(0.0, &[], &frame_events(&times), 1, 1);
        assert_eq!(observed.frame_samples, Some(1000));
        assert_eq!(observed.frame_p99_ms, Some(10.0)); // rank 990
        assert_eq!(observed.frame_p99_9_ms, Some(40.0)); // rank 999
        assert_eq!(observed.frame_over_16_6_rate, Some(10.0 / 1000.0));
        assert_eq!(observed.frame_over_33_3_rate, Some(2.0 / 1000.0));
        // Non-finite frame times are ignored rather than poisoning the percentile.
        let noisy = observe(0.0, &[], &frame_events(&[10.0, f64::NAN, 40.0]), 1, 1);
        assert_eq!(noisy.frame_samples, Some(2));
        assert_eq!(noisy.frame_p99_9_ms, Some(40.0));
    }

    #[test]
    fn frame_inclusive_score_adds_the_frame_terms_to_the_runnable_score() {
        let frames = frame_events(&[10.0, 20.0, 40.0, 60.0]);
        let observed = observe(0.0, &[], &frames, 1, 1);
        // >16.6: 3 x 1, >33.3: 2 x 20, >50: 1 x 100; no interval records contribute.
        assert_eq!(observed.runnable_score_total, 0.0);
        assert_eq!(observed.score_total, 143.0);
    }

    #[test]
    fn frames_at_every_level_use_the_frame_inclusive_score_and_correlate_frame_metrics() {
        let report = evaluate(
            LEVELS
                .iter()
                .enumerate()
                .map(|(i, l)| with_frames(*l, i as f64 * 10.0, 20.0 + i as f64, 600))
                .collect(),
            true,
        )
        .unwrap();
        assert_eq!(report["score_basis"], "runnable_plus_frames");
        assert_eq!(report["monotonic_in_degradation"], true);
        assert_eq!(report["spearman_score_vs"]["frame_p99_9"], 1.0);
        assert_eq!(report["spearman_score_vs"]["frame_over_33_3_rate"], 1.0);
    }

    #[test]
    fn require_frames_rejects_any_level_without_usable_frames() {
        let mut observations = LEVELS
            .iter()
            .enumerate()
            .map(|(i, l)| with_frames(*l, i as f64 * 10.0, 20.0, 600))
            .collect::<Vec<_>>();
        assert!(evaluate(observations.clone(), true).is_ok());
        // One level with no frames, another with an empty stream.
        observations[3].frame_samples = None;
        observations[4].frame_samples = Some(0);
        let error = evaluate(observations.clone(), true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--require-frames"), "{error}");
        assert!(error.contains("4") && error.contains("8"), "{error}");
        // Without the flag, mixed availability falls back to a consistent runnable-only series.
        let report = evaluate(observations, false).unwrap();
        assert_eq!(report["score_basis"], "runnable_only");
        assert!(report["spearman_score_vs"]["frame_p99"].is_null());
    }

    #[test]
    fn score_and_direct_frame_metric_can_invert() {
        // Monotonic score, but frame p99.9 falls as degradation rises: a rank inversion the
        // correlation exposes instead of hiding behind the score.
        let report = evaluate(
            LEVELS
                .iter()
                .enumerate()
                .map(|(i, l)| with_frames(*l, i as f64 * 10.0, 100.0 - i as f64, 600))
                .collect(),
            true,
        )
        .unwrap();
        assert_eq!(report["monotonic_in_degradation"], true);
        assert_eq!(report["spearman_score_vs"]["frame_p99_9"], -1.0);
    }

    #[test]
    fn frame_streams_load_from_the_canonical_name_then_the_legacy_alias() {
        let dir = std::env::temp_dir().join(format!("stutter-score-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_frames(&dir).unwrap().is_empty());
        std::fs::write(
            dir.join("frame_events.json"),
            "{\"elapsed_ms\":1,\"frametime_ms\":12.5}\n\n",
        )
        .unwrap();
        assert_eq!(load_frames(&dir).unwrap().len(), 1);
        std::fs::write(
            dir.join("frame_correlation.json"),
            "{\"elapsed_ms\":1,\"frametime_ms\":10.0}\n{\"elapsed_ms\":2,\"frametime_ms\":11.0}\n",
        )
        .unwrap();
        assert_eq!(load_frames(&dir).unwrap().len(), 2);
        std::fs::write(dir.join("frame_correlation.json"), "not json\n").unwrap();
        assert!(load_frames(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn with_identity(
        mut observation: LevelObservation,
        source: IdentitySource,
    ) -> LevelObservation {
        observation.identity = Some(TargetIdentity {
            pid: 100,
            starttime_ticks: 1111,
            source,
        });
        observation
    }

    #[test]
    fn require_frames_rejects_the_legacy_target_heuristic_and_reports_the_target() {
        let framed = |source| {
            LEVELS
                .iter()
                .enumerate()
                .map(|(i, l)| with_identity(with_frames(*l, i as f64 * 10.0, 20.0, 600), source))
                .collect::<Vec<_>>()
        };
        let error = evaluate(framed(IdentitySource::LegacySampleHeuristic), true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("legacy"), "{error}");
        let report = evaluate(framed(IdentitySource::TreeRoot), true).unwrap();
        assert_eq!(report["targets"][0]["target_pid"], 100);
        assert_eq!(report["targets"][0]["target_starttime_ticks"], 1111);
        assert_eq!(report["targets"][0]["target_identity_source"], "tree_root");
        // Exploratory (non-strict) use may still label and use the heuristic.
        let report = evaluate(framed(IdentitySource::LegacySampleHeuristic), false).unwrap();
        assert_eq!(
            report["targets"][5]["target_identity_source"],
            "legacy_sample_heuristic"
        );
    }
}
