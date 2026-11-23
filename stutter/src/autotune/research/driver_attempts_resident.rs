//! The resident runtime's observation of each attempt, kept and sealed beside its study: the raw windows, the resident
//! observation G3 stands on and the paired runnable-latency record G5 stands on, each derived from what the runtime
//! counted and written only when its real verifier accepts it.

use super::{
    super::{
        child_study::{ChildStudy, Stage},
        report_store::write_new,
    },
    DriverStudyAttempts, STUDIES_DIR, dir_name, open_dir,
};

impl DriverStudyAttempts {
    /// Share the resident observer with the runtime: each attempt's observed window is kept beside its study.
    pub fn with_resident(
        mut self,
        resident: super::super::resident_observer::SharedResident,
    ) -> Self {
        self.resident = Some((resident, 0));
        self
    }

    /// Observe the attested game process of the running attempt: the runtime's interval and GPU events are counted
    /// against it from here on.
    pub(super) fn arm_resident(&mut self, now: u64) {
        let Some(target) = self.host.game_process() else {
            return;
        };
        let attempt = self.host.attempt_index();
        if let Some((resident, latest)) = self.resident.as_mut() {
            *latest = now;
            if let Ok(mut observer) = resident.lock() {
                observer.arm(target, attempt, now);
            }
        }
    }

    /// Keep every attempt's observed window beside the study as raw, immutable counters (write-once; a conflicting
    /// file means the windows cannot be recorded and the study simply has none).
    pub(super) fn record_resident(
        &self,
        study: &ChildStudy,
    ) -> anyhow::Result<Vec<super::super::resident_observer::AttemptResident>> {
        let Some((resident, latest)) = &self.resident else {
            return Ok(Vec::new());
        };
        let windows = resident
            .lock()
            .map_err(|_| anyhow::anyhow!("the resident observer is poisoned"))?
            .freeze(*latest);
        if windows.is_empty() {
            return Ok(windows);
        }
        let rows: Vec<serde_json::Value> = windows
            .iter()
            .map(super::super::resident_observer::window_record)
            .collect();
        let studies = open_dir(&self.campaign, STUDIES_DIR)?;
        let dir = open_dir(&studies, dir_name(study))?;
        let bytes = serde_json::to_vec_pretty(&rows)?;
        match write_new(&dir, super::super::resident_observer::WINDOWS_FILE, &bytes) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                anyhow::ensure!(
                    super::super::report_store::read_private(
                        &dir,
                        super::super::resident_observer::WINDOWS_FILE,
                        self.parent.uid
                    )? == bytes,
                    "resident windows not recorded: the existing windows differ"
                );
            }
            Err(error) => return Err(error.context("resident windows not recorded")),
        }
        resident
            .lock()
            .map_err(|_| anyhow::anyhow!("the resident observer is poisoned"))?
            .acknowledge(windows.len());
        Ok(windows)
    }

    /// The paired runnable-latency record G5 stands on, derived from the windows the runtime counted: each planned
    /// pair's off and on attempt as its canonical population (the part inside the attempt), the dropped events and any
    /// target change, and the guard facts read from the production objective-signal builder. Written only when the real
    /// verifier accepts it for every planned pair; anything less leaves no record and the action has no measurement.
    pub(super) fn seal_action_latency(
        &self,
        study: &ChildStudy,
        study_dir: &std::fs::File,
        intent: &super::super::ResearchIntentV1,
        windows: &[super::super::resident_observer::AttemptResident],
        evidence: &std::collections::BTreeMap<u32, super::super::report::AttemptEvidenceV1>,
    ) -> anyhow::Result<()> {
        use super::super::{
            action_latency,
            child_verify_rules::processes_of,
            evidence_producers::{
                LatencyWindowBuilder, WindowGuards, action_latency_document, latency_pair,
            },
            treatment::{Treatment, treatments_of},
        };
        if !matches!(study.stage, Stage::Action(_)) || windows.is_empty() {
            return Ok(());
        }
        let Some(schedule) = treatments_of(intent).ok().flatten() else {
            return Ok(());
        };
        let processes = processes_of(evidence);
        let window_json = |slot: u32| -> Option<serde_json::Value> {
            let process = processes.iter().find(|p| p.slot == slot)?;
            let window = windows.iter().find(|w| {
                w.attempt == Some(slot)
                    && w.target.pid == process.pid
                    && w.target.start_ticks == process.start_ticks
            })?;
            let guards = window.guards?;
            let mut builder = LatencyWindowBuilder::from_population(
                window.started_unix.max(process.started_unix),
                window.histogram.clone(),
                window.max_ns,
            )?;
            builder.lost(window.lost_events.saturating_add(window.invalid_intervals));
            if window.target_changed {
                builder.target_changed();
            }
            Some(builder.finish(
                window.ended_unix.min(process.completed_unix),
                WindowGuards {
                    foreground_p99_ms: guards.frame_p99_ms,
                    foreground_samples: guards.frame_samples,
                    foreground_over_5ms: guards.foreground_over_5ms,
                    thermal_degraded: guards.thermal_degraded,
                    thermal_throttle_count: guards.thermal_throttle_count,
                    cpu_power_limited: guards.cpu_power_limited,
                    gpu_power_limited: guards.gpu_power_limited,
                },
            ))
        };
        let mut pairs = Vec::new();
        for block in 0..schedule.len() / 2 {
            let (a, b) = (block as u32 * 2, block as u32 * 2 + 1);
            let (off, on) = if schedule[a as usize] == Treatment::Off {
                (a, b)
            } else {
                (b, a)
            };
            let (Some(off_process), Some(on_process)) = (
                processes.iter().find(|p| p.slot == off),
                processes.iter().find(|p| p.slot == on),
            ) else {
                continue;
            };
            let (Some(off_window), Some(on_window)) = (window_json(off), window_json(on)) else {
                return Ok(());
            };
            pairs.push(latency_pair(
                (off_process, off_window),
                (on_process, on_window),
            ));
        }
        let bytes = action_latency_document(pairs);
        if action_latency::verify(&bytes, &schedule, &processes).is_err() {
            return Ok(());
        }
        match write_new(study_dir, action_latency::FILE, &bytes) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The sealed resident observation G3 stands on, derived from the windows the runtime counted and the facts the
    /// kernel and the frame source gave of each attempt: the cgroup the window opened with, the renderer binding the
    /// attempt's own telemetry receipt shows, and owners attributed by the game's own interval records. It is written
    /// only when the real verifier accepts it for every completed attempt; a campaign without it has no G3.
    pub(super) fn seal_resident_observation(
        &self,
        study_dir: &std::fs::File,
        windows: &[super::super::resident_observer::AttemptResident],
        evidence: &std::collections::BTreeMap<u32, super::super::report::AttemptEvidenceV1>,
    ) -> anyhow::Result<()> {
        use super::super::{
            child_verify_rules::processes_of,
            resident_observation,
            resident_observer::{AttemptResident, resident_entry},
        };
        let processes = processes_of(evidence);
        if windows.is_empty() || processes.is_empty() {
            return Ok(());
        }
        let telemetry: serde_json::Value = super::super::report_store::read_private(
            study_dir,
            "telemetry-receipts.json",
            self.parent.uid,
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
        let mut entries = Vec::new();
        for process in &processes {
            let Some(window) = windows.iter().find(|window| {
                window.attempt == Some(process.slot)
                    && window.target.pid == process.pid
                    && window.target.start_ticks == process.start_ticks
            }) else {
                return Ok(());
            };
            let bound = telemetry.as_array().is_some_and(|receipts| {
                receipts.iter().any(|receipt| {
                    receipt["attempt"].as_u64() == Some(u64::from(process.slot))
                        && receipt["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                        && receipt["start_ticks"].as_u64() == Some(process.start_ticks)
                        && receipt["source_bound_to_game"] == true
                        && receipt["source_healthy"] == true
                        && receipt["snapshots"].as_u64().unwrap_or(0) > 0
                })
            });
            // The observation covers the part of the window inside the attempt.
            let clamped = AttemptResident {
                started_unix: window.started_unix.max(process.started_unix),
                ended_unix: window.ended_unix.min(process.completed_unix),
                ..window.clone()
            };
            entries.push(resident_entry(
                &clamped,
                process,
                &window.cgroup,
                bound,
                window.cpu_samples > 0,
                false,
            ));
        }
        let bytes = super::super::evidence_producers::resident_observation_document(entries);
        if resident_observation::verify(&bytes, &processes).is_err() {
            return Ok(());
        }
        match write_new(study_dir, resident_observation::FILE, &bytes) {
            Ok(()) => Ok(()),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
