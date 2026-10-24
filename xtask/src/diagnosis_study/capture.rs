//! One study capture: run the predeclared scenario through the shared capture pipeline, then
//! take the direct effect metric from the target's own recording (never from a diagnosis).
use std::{collections::BTreeSet, fs, path::Path};

use serde_json::{Value, json};
use stutter::api::recorder::IntervalRecord;

use super::{
    metric::merged_runnable_p99_ns,
    model::{Metric, StudyConfig},
    plan::Slot,
    store::{RunRecord, StudyDir},
};
use crate::{
    diagnosis_calibration::load_corpus,
    score_evaluation::load_observation,
    validation_campaign::{
        scenarios::exec::{CaptureHost, CaptureRequest, CaptureResult, run_capture},
        state::{CampaignDir, unix_ms},
    },
};

/// The primary (direct, user-facing) metric of an accepted capture.
pub fn primary_metric(metric: Metric, raw: &Path, host_metrics: &Value) -> Result<f64, String> {
    let value = match metric {
        Metric::TargetElapsedMs => host_metrics["target_elapsed_ms"].as_f64(),
        Metric::RunnableP99Ns | Metric::FrameP99Ms => {
            let observation = load_observation(0.0, raw, true).map_err(|e| format!("{e:#}"))?;
            match metric {
                Metric::RunnableP99Ns => {
                    let pid = observation.identity.map(|identity| identity.pid);
                    let records = fs::read_to_string(raw.join("interval.json"))
                        .map_err(|e| e.to_string())?
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(serde_json::from_str::<IntervalRecord>)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    pid.and_then(|pid| merged_runnable_p99_ns(&records, pid))
                }
                _ => observation.frame_p99_ms,
            }
        }
    };
    value
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("the recording has no {}", metric.name()))
}

pub struct Session<'a> {
    pub study: &'a StudyDir,
    pub study_id: &'a str,
    pub base_commit: &'a str,
    pub config: &'a StudyConfig,
    pub companion: &'a CampaignDir,
}

fn published_ids(study: &StudyDir) -> BTreeSet<String> {
    load_corpus(&study.fixtures())
        .map(|(fixtures, _)| {
            fixtures
                .iter()
                .filter_map(|f| f.meta.as_ref().map(|m| m.capture_id.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Runs one capture for `slot` and returns the record to append. `seq` is unique per attempt
/// of the whole study and names the attempt directory the privileged runner writes into.
pub fn capture_once(
    session: &Session<'_>,
    host: &mut dyn CaptureHost,
    slot: &Slot,
    seq: u64,
    failed_before: usize,
    integrity: &dyn Fn() -> bool,
) -> anyhow::Result<RunRecord> {
    let spec = session.config.scenario(&slot.scenario)?;
    let raw_dir = session
        .companion
        .attempt_dir("corpus", u32::try_from(seq)?)
        .join("captures/0001/raw");
    fs::create_dir_all(session.study.fixtures())?;
    let raw_capture_id = format!("{}-{}-{seq}-{}", slot.scenario, slot.pair, unix_ms());
    let published = published_ids(session.study);
    let request = CaptureRequest {
        spec: &spec,
        campaign_id: session.study_id,
        attempt: u32::try_from(failed_before + 1)?,
        raw_capture_id: &raw_capture_id,
        source_commit: session.base_commit,
        raw_dir: &raw_dir,
        fixtures_dir: &session.study.fixtures(),
        published_ids: &published,
        fingerprint_matches: integrity,
    };
    let result = run_capture(host, &request);
    let host_metrics = host.study_metrics();
    let mut record = RunRecord {
        seq,
        scenario: slot.scenario.clone(),
        family: slot.family,
        role: slot.role,
        pair: slot.pair,
        accepted: false,
        stage: None,
        reasons: Vec::new(),
        capture_id: None,
        fixture: None,
        metrics: Value::Null,
        unix_ms: unix_ms(),
    };
    match result {
        CaptureResult::Rejected { stage, reasons } => {
            record.stage = Some(stage);
            record.reasons = reasons;
            record.metrics = json!({"supporting": host_metrics});
        }
        CaptureResult::Accepted {
            fixture_dir,
            capture_id,
        } => {
            let primary = match slot.family.and_then(|f| session.config.family(f)) {
                Some(def) if slot.role != super::store::Role::Mixed => {
                    primary_metric(def.metric, &raw_dir, &host_metrics).map(Some)
                }
                _ => Ok(None),
            };
            match primary {
                Ok(primary) => {
                    record.accepted = true;
                    record.capture_id = Some(capture_id);
                    record.fixture = fixture_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned());
                    record.metrics = json!({"primary": primary, "supporting": host_metrics});
                }
                Err(reason) => {
                    // A capture without its direct metric cannot count: take it back.
                    let _ = fs::remove_dir_all(&fixture_dir);
                    record.stage = Some("metric".to_owned());
                    record.reasons = vec![reason];
                }
            }
        }
    }
    Ok(record)
}
