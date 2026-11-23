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
    /// Archive the controller's own journal records and the privileged worker's own audit events of the action this
    /// study ran, beside the study, in the formats the replay joins to the gate's receipts. Write-once; an action
    /// study that applied nothing has nothing to archive.
    pub(super) fn archive_provenance(
        &self,
        study: &ChildStudy,
        study_dir: &std::fs::File,
        evidence: &std::collections::BTreeMap<u32, super::super::report::AttemptEvidenceV1>,
    ) -> anyhow::Result<()> {
        use super::super::{controller_provenance, evidence_archive, worker_provenance};
        let (Some((archive, audit_path)), Stage::Action(action)) = (&self.provenance, study.stage)
        else {
            return Ok(());
        };
        let processes = super::super::child_verify_rules::processes_of(evidence);
        let records = archive
            .lock()
            .map_err(|_| anyhow::anyhow!("the controller journal archive is poisoned"))?
            .for_processes(&processes);
        let audit = evidence_archive::worker_audit_for(audit_path, action, &processes);
        // A worker audit this process is not allowed to read is a stated reason, never a silent absence: the action then has
        // no independent corroboration and does not qualify, and the report can say why.
        if let Err(kind) = evidence_archive::worker_audit_readable(audit_path)
            && !records.is_empty()
        {
            let note = serde_json::to_vec(&serde_json::json!({
                "reason": format!("{kind:?}"), "path": audit_path.display().to_string(),
            }))?;
            let _ = write_new(study_dir, "worker-audit-unavailable.json", &note);
        }
        for (name, bytes) in [
            (controller_provenance::FILE, serde_json::to_vec(&records)?),
            (worker_provenance::FILE, serde_json::to_vec(&audit)?),
        ] {
            if records.is_empty() {
                break;
            }
            match write_new(study_dir, name, &bytes) {
                Ok(()) => {}
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// The planner's own observation that justifies an abstention: for every completed attempt, how often the planner
    /// evaluated the game process and that it never offered the authorized action. Derived from the runtime's decision
    /// stream and written only when the real verifier accepts it for every completed attempt.
    pub(super) fn seal_planner_observation(
        &self,
        study: &ChildStudy,
        study_dir: &std::fs::File,
        windows: &[super::super::resident_observer::AttemptResident],
        evidence: &std::collections::BTreeMap<u32, super::super::report::AttemptEvidenceV1>,
    ) -> anyhow::Result<()> {
        use super::super::{ResearchAction, planner_observation};
        let Stage::Action(action) = study.stage else {
            return Ok(());
        };
        let processes = super::super::child_verify_rules::processes_of(evidence);
        let mut attempts = Vec::new();
        for process in &processes {
            let Some(window) = windows.iter().find(|w| {
                w.attempt == Some(process.slot)
                    && w.target.pid == process.pid
                    && w.target.start_ticks == process.start_ticks
            }) else {
                return Ok(());
            };
            let candidates = match action {
                ResearchAction::CpuAffinityProfile => window.affinity_candidates,
                ResearchAction::CgroupPlacement => window.cgroup_candidates,
            };
            let direct =
                window.cpu_samples > 0 && window.lost_events == 0 && window.invalid_intervals == 0;
            attempts.push(serde_json::json!({
                "slot": process.slot,
                "process": { "pid": process.pid, "start_ticks": process.start_ticks, "uid": process.uid },
                "start_unix": window.started_unix.max(process.started_unix),
                "end_unix": window.ended_unix.min(process.completed_unix),
                "evaluations": window.evaluations, "candidates": candidates,
                "lost": window.lost_events.saturating_add(window.invalid_intervals),
                "source_quality": if direct { "direct" } else { "derived" },
            }));
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": 1, "action": format!("{action:?}"), "attempts": attempts,
        }))?;
        if planner_observation::verify(&bytes, action, &processes).is_err() {
            return Ok(());
        }
        match write_new(study_dir, planner_observation::FILE, &bytes) {
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

    /// Archive the controller's and the worker's own records of an action beside its study.
    pub fn with_provenance(
        mut self,
        archive: super::super::evidence_archive::SharedJournalArchive,
        worker_audit: std::path::PathBuf,
    ) -> Self {
        self.provenance = Some((archive, worker_audit));
        self
    }

    /// Merge the kernel's witness of the attempt's game process into what was seen so far (read at most every few
    /// seconds), with the hashes of the tool's own two builds when the factor is told apart by content.
    pub(super) fn merge_witness(
        &mut self,
        factor: super::super::ResearchFactor,
        index: u32,
        treatment: super::super::Treatment,
        key: super::super::feature_proof::AttemptProcessKey,
        now: u64,
    ) {
        let stale = self
            .feature_witness
            .get(&index)
            .is_none_or(|seen| now >= seen.3 + 5);
        let watch = super::super::feature_proof::watched(factor);
        let Some(witness) = stale
            .then(|| (self.witness_of)(u32::from(key.0), &watch))
            .flatten()
        else {
            return;
        };
        let builds = super::super::feature_proof::tool_builds(factor).map(|(on, off)| {
            (
                self.host.tool_file_sha256(on),
                self.host.tool_file_sha256(off),
            )
        });
        let seen =
            self.feature_witness
                .entry(index)
                .or_insert((treatment, key, Default::default(), now));
        seen.2.modules.extend(witness.modules);
        seen.2.devices.extend(witness.devices);
        seen.2.ancestors.extend(witness.ancestors);
        for (name, hashes) in witness.mapped {
            seen.2.mapped.entry(name).or_default().extend(hashes);
        }
        if let Some((on, off)) = builds {
            seen.2.tool_on_sha256 = on;
            seen.2.tool_off_sha256 = off;
        }
        seen.3 = now;
    }

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
