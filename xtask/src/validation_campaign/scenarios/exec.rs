//! One capture, end to end, against a [`CaptureHost`]: preflight, launch the target, apply and
//! verify the interventions, record, sanitize, generate the fixture from the scenario, validate,
//! accept or reject. Every path stops all processes it started; a rejected capture publishes
//! nothing; an accepted one appears atomically.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::{
    capture::{CaptureEvidence, InterventionReport, accept},
    fixture::{
        CaptureRecord, HostFacts, count_artifacts, fixture_name, render, sanitized_capture_id,
    },
    model::ScenarioSpec,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetIdentity {
    pub pid: u32,
    pub starttime_ticks: u64,
}

#[derive(Clone, Debug, Default)]
pub struct RecordSummary {
    pub valid: bool,
    pub drops: u64,
    pub write_errors: u64,
    pub frame_samples: u64,
}

/// Side effects of a capture. The real implementation drives vkmark/fio/network traffic, the
/// privileged runner (burst, recording) and the sanitizer; tests use a scripted fake.
pub trait CaptureHost {
    fn preflight(&mut self, spec: &ScenarioSpec) -> Result<(), String>;
    fn start_target(&mut self, spec: &ScenarioSpec) -> Result<TargetIdentity, String>;
    fn start_interventions(
        &mut self,
        spec: &ScenarioSpec,
        target: TargetIdentity,
    ) -> Result<(), String>;
    fn record(
        &mut self,
        spec: &ScenarioSpec,
        target: TargetIdentity,
        raw: &Path,
    ) -> Result<RecordSummary, String>;
    /// Independent telemetry about each applied intervention, gathered while it is still active.
    fn verify_interventions(&mut self, spec: &ScenarioSpec, raw: &Path) -> Vec<InterventionReport>;
    fn stop_all(&mut self);
    fn sanitize(&mut self, raw: &Path, out: &Path, name: &str) -> Result<(), String>;
    /// `data_quality.level` of the recording (a statement about the data, not a diagnosis).
    fn data_quality(&mut self, dir: &Path) -> Result<String, String>;
    fn validate(&mut self, dir: &Path) -> Result<(), String>;
    fn facts(&self) -> (HostFacts, String);
}

pub struct CaptureRequest<'a> {
    pub spec: &'a ScenarioSpec,
    pub campaign_id: &'a str,
    pub attempt: u32,
    /// Unique per run (the raw recording's own capture id).
    pub raw_capture_id: &'a str,
    pub source_commit: &'a str,
    pub raw_dir: &'a Path,
    /// Where accepted fixtures are published (`.../corpus/fixtures`).
    pub fixtures_dir: &'a Path,
    pub published_ids: &'a BTreeSet<String>,
    /// Re-checked after the recording: source, binaries and manifest are unchanged.
    pub fingerprint_matches: &'a dyn Fn() -> bool,
}

#[derive(Debug)]
pub enum CaptureResult {
    Accepted {
        fixture_dir: PathBuf,
        capture_id: String,
    },
    Rejected {
        stage: String,
        reasons: Vec<String>,
    },
}

fn rejected(stage: &str, reason: impl Into<String>) -> CaptureResult {
    CaptureResult::Rejected {
        stage: stage.to_owned(),
        reasons: vec![reason.into()],
    }
}

pub fn run_capture(host: &mut dyn CaptureHost, request: &CaptureRequest<'_>) -> CaptureResult {
    let spec = request.spec;
    if let Err(error) = host.preflight(spec) {
        return rejected("preflight", error);
    }
    let result = capture_inner(host, request);
    // Whatever happened, nothing this capture started may outlive it.
    host.stop_all();
    match result {
        Ok(evidence) => finish(host, request, evidence),
        Err(rejection) => rejection,
    }
}

/// Everything that needs the target and interventions alive.
fn capture_inner(
    host: &mut dyn CaptureHost,
    request: &CaptureRequest<'_>,
) -> Result<(RecordSummary, Vec<InterventionReport>, bool), CaptureResult> {
    let spec = request.spec;
    let target = host.start_target(spec).map_err(|e| rejected("launch", e))?;
    host.start_interventions(spec, target)
        .map_err(|e| rejected("intervention", e))?;
    let summary = host
        .record(spec, target, request.raw_dir)
        .map_err(|e| rejected("record", e))?;
    let reports = host.verify_interventions(spec, request.raw_dir);
    let identity_valid = target.pid != 0 && target.starttime_ticks != 0;
    Ok((summary, reports, identity_valid))
}

fn finish(
    host: &mut dyn CaptureHost,
    request: &CaptureRequest<'_>,
    captured: (RecordSummary, Vec<InterventionReport>, bool),
) -> CaptureResult {
    let (summary, reports, identity_valid) = captured;
    let spec = request.spec;
    let capture_id = sanitized_capture_id(
        request.campaign_id,
        &spec.id,
        request.attempt,
        request.raw_capture_id,
    );
    let name = fixture_name(&spec.id, &capture_id);
    let staging = request.fixtures_dir.join(format!(".staging-{name}"));
    let _ = fs::remove_dir_all(&staging);
    let mut evidence = CaptureEvidence {
        target_identity_valid: identity_valid,
        recording_valid: summary.valid,
        drops: summary.drops,
        write_errors: summary.write_errors,
        interventions: reports,
        fingerprint_matches: (request.fingerprint_matches)(),
        capture_id_unique: !request.published_ids.contains(&capture_id),
        frame_samples: summary.frame_samples,
        ..CaptureEvidence::default()
    };
    // Sanitize, then describe the capture from the *scenario*; the sanitizer and the
    // validators only gate it.
    if host.sanitize(request.raw_dir, &staging, &name).is_ok() {
        evidence.sanitizer_ok = true;
        let counts = count_artifacts(&staging);
        evidence.frame_samples = evidence
            .frame_samples
            .max(counts.get("frames").copied().unwrap_or(0));
        let (facts, probes) = host.facts();
        if let Ok(quality) = host.data_quality(&staging) {
            let record = CaptureRecord {
                spec,
                campaign_id: request.campaign_id,
                attempt: request.attempt,
                raw_capture_id: request.raw_capture_id,
                source_commit: request.source_commit,
                host: &facts,
                probe_set: &probes,
                drop_counters: summary.drops,
                data_quality: &quality,
                artifact_counts: &counts,
            };
            if let Ok(text) = render(&record)
                && fs::write(staging.join("fixture.toml"), text).is_ok()
            {
                evidence.fixture_valid = host.validate(&staging).is_ok();
            }
        }
    }
    match accept(spec, &evidence) {
        Err(reasons) => {
            let _ = fs::remove_dir_all(&staging);
            CaptureResult::Rejected {
                stage: "acceptance".to_owned(),
                reasons,
            }
        }
        Ok(()) => {
            let final_dir = request.fixtures_dir.join(&name);
            match fs::rename(&staging, &final_dir) {
                Ok(()) => CaptureResult::Accepted {
                    fixture_dir: final_dir,
                    capture_id,
                },
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    rejected("publish", error.to_string())
                }
            }
        }
    }
}
