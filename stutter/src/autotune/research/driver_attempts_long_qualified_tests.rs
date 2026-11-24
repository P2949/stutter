use super::*;

/// The measurement the study's objective needs, built by the evidence collectors from a tail the action shortened
/// in every pair: with it sealed beside the receipts the same shared judgement that refused the study without it now
/// qualifies it, and the replay (which recomputes the tail from the populations) agrees with the live outcome.
#[test]
fn a_long_action_study_with_its_sealed_latency_measurement_qualifies_and_replays() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::{
        action_latency,
        child_verify::AttemptProcess,
        treatment::{Treatment, treatments_of},
    };
    let (root, mut runner, study, gate, parent) = build(&unit().definition_sha256);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
    let schedule = treatments_of(&intent).unwrap().unwrap();
    let process = |slot: u32| AttemptProcess {
        slot,
        pid: attempt_process(slot).pid,
        start_ticks: attempt_process(slot).start_ticks,
        uid: attempt_process(slot).uid,
        started_unix: 5_000,
        completed_unix: 6_200,
    };
    let processes: Vec<_> = (0..schedule.len() as u32).map(process).collect();
    // The gate holds the action 300 s in each treated attempt: shown held one second after the controller acts (at
    // 5_100 plus ten seconds per attempt), restored 300 s later.
    let receipts: Vec<serde_json::Value> = processes
        .iter()
        .filter(|p| schedule[p.slot as usize] == Treatment::On)
        .map(|p| {
            serde_json::json!({ "pid": p.pid, "start_ticks": p.start_ticks,
                                "applied_unix": 5_101 + 10 * u64::from(p.slot),
                                "settled_unix": 5_401 + 10 * u64::from(p.slot) })
        })
        .collect();
    // The windows the collectors counted: the tail is the action's effect in an attempt treated On.
    let windows = |off_tail: u64, on_tail: u64| -> Vec<_> {
        processes
            .iter()
            .map(|p| {
                let tail = if schedule[p.slot as usize] == Treatment::On {
                    on_tail
                } else {
                    off_tail
                };
                super::latency_fixtures::fixture_window(
                    attempt_process(p.slot),
                    p.slot,
                    5_000,
                    6_200,
                    tail,
                )
            })
            .collect()
    };
    let record = |off_tail: u64, on_tail: u64| -> Vec<u8> {
        crate::autotune::research::driver_attempts::resident::latency_document(
            &schedule,
            &processes,
            &receipts,
            &windows(off_tail, on_tail),
        )
        .expect("the collectors' windows make a valid record")
    };
    // The raw windows the record derives from, beside the study, as the resident runtime keeps them.
    let raw: Vec<_> = windows(20_000_000, 1_500_000)
        .iter()
        .map(crate::autotune::research::resident_observer::window_record)
        .collect();
    let raw_file = path.join(crate::autotune::research::resident_observer::WINDOWS_FILE);
    std::fs::write(&raw_file, serde_json::to_vec(&raw).unwrap()).unwrap();
    std::fs::set_permissions(&raw_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = path.join(action_latency::FILE);
    std::fs::write(&file, record(20_000_000, 1_500_000)).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let outcome = run_acting(&mut runner, &study, &gate);
    assert!(
        outcome.action_metric_valid,
        "the sealed measurement is a valid one"
    );
    let run = judge(&study, &outcome);
    assert_eq!(
        (run.result, run.reason),
        (StudyResult::Qualified, ""),
        "{outcome:?}"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
    let disposition = crate::autotune::research::child_ledger::Disposition {
        study_sha256: study.study_sha256.clone(),
        profile: study.profile,
        stage: study.stage,
        result: Some(StudyResult::Qualified),
        reason: String::new(),
    };
    let dir = std::fs::File::open(&path).unwrap();
    let replayed = crate::autotune::research::child_judgement::verify_qualification(
        &dir,
        crate::syscall::geteuid(),
        &parent,
        &disposition,
        &report,
        true,
        true,
    );
    assert!(replayed.is_ok(), "{replayed:?}");
    // Replay: the same function over the same bytes, and a worse tail is a measurement that is still valid but
    // decides Revert, never Keep.
    let bytes = std::fs::read(&file).unwrap();
    let result = action_latency::verify(&bytes, &schedule, &processes).unwrap();
    assert_eq!(
        (result.pairs, result.decision),
        (4, action_latency::Decision::Keep)
    );
    let worse =
        action_latency::verify(&record(1_500_000, 20_000_000), &schedule, &processes).unwrap();
    assert_eq!(worse.decision, action_latency::Decision::Revert);
    // A record that names another process cannot stand for these attempts.
    let mut stranger = process(0);
    stranger.pid = stutter_core::ids::Pid::new(1);
    let mut moved = processes.clone();
    moved[0] = stranger;
    assert!(action_latency::verify(&bytes, &schedule, &moved).is_err());
}

/// No receipts is not, by itself, a justified abstention: without the planner's own observation of every attempt a
/// study in which nothing was applied is simply an action study with no evidence.
#[test]
fn nothing_applied_without_the_planners_observation_is_not_a_justified_abstention() {
    let (_root, mut runner, study, _gate, _parent) = build(&unit().definition_sha256);
    // A controller that never ran leaves no receipts either: nothing here ever lets the gate act.
    let outcome = loop {
        if let Some(outcome) = runner.poll(&study, 100).unwrap() {
            break outcome;
        }
    };
    assert!(!outcome.action_not_indicated, "no planner observation");
    let run = judge(&study, &outcome);
    assert_ne!(run.reason, "ActionNotIndicated");
}

/// What the resident runtime observed of each attempt's game process is kept beside the study as raw counters, one
/// window per attempt on its own process, and survives only as that: nothing is stated, derived or decided here.
#[test]
fn every_attempts_observed_window_is_kept_beside_the_study_as_raw_counters() {
    use crate::{
        autotune::research::resident_observer::{SharedResident, WINDOWS_FILE},
        metrics::{IntervalRecord, LatencyHistogram},
        process_tree::TaskClass,
    };
    let (root, runner, study, _gate, _parent) = build(&unit().definition_sha256);
    // The fixture sessions sit at synthetic unix times: the observers stamp their epochs from a clock that walks them.
    crate::autotune::research::resident_observer::TEST_CLOCK
        .with(|clock| clock.set(Some(super::latency_fixtures::clock())));
    let resident: SharedResident = Default::default();
    let mut runner = runner.with_resident(resident.clone());
    let (sighted, _, _) = RIG.with(|rig| rig.borrow().clone().unwrap());
    let outcome = 'run: {
        for _ in 0..20_000 {
            if let Some(outcome) = runner.poll(&study, 100).unwrap() {
                break 'run outcome;
            }
            // The runtime's interval event for the game process the host attested.
            if let Some(process) = *sighted.lock().unwrap() {
                let mut histogram = LatencyHistogram::new();
                for _ in 0..50 {
                    histogram.record(700_000);
                }
                resident.lock().unwrap().observe_intervals(
                    &[IntervalRecord {
                        process_pid: Some(process.pid.as_u32()),
                        class: TaskClass::Game,
                        samples: 50,
                        max_ns: 700_000,
                        histogram: histogram.snapshot(),
                        ..Default::default()
                    }],
                    0,
                );
            }
        }
        panic!("the study never ended");
    };
    assert_eq!(outcome.completed, 8);
    let path = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join(WINDOWS_FILE);
    let windows: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let windows = windows.as_array().unwrap();
    assert_eq!(windows.len(), 8, "one window per attempt");
    let pids: std::collections::BTreeSet<_> =
        windows.iter().map(|w| w["pid"].as_u64().unwrap()).collect();
    assert_eq!(pids.len(), 8, "each on its own game process");
    assert!(windows.iter().all(|w| {
        w["lost_events"] == 0
            && w["invalid_intervals"] == 0
            && w["cpu_samples"].as_u64().unwrap() > 0
            && w["histogram"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b["count"].as_u64().unwrap())
                .sum::<u64>()
                > 0
    }));
    // Persisted and acknowledged: nothing is left to write twice.
    assert!(resident.lock().unwrap().freeze(1).is_empty());
}

/// The sealed resident observation is derived from the raw windows and the attempt's own facts, and written only
/// when the real verifier accepts it for every completed attempt: an unbound renderer, a window missing for an
/// attempt or a sparse one leaves no record, and a campaign without it has no G3.
#[test]
fn the_sealed_resident_observation_is_derived_only_when_every_attempt_verifies() {
    use std::os::unix::fs::PermissionsExt;

    use crate::{
        autotune::research::{
            report::AttemptEvidenceV1,
            resident_observation,
            resident_observer::{AttemptResident, ResidentObserver},
        },
        metrics::{IntervalRecord, LatencyHistogram},
        process_tree::TaskClass,
    };
    let (root, mut runner, study, _gate, _parent) = build(&unit().definition_sha256);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let path = root.path().join("studies").join(&study.study_sha256[..16]);
    let dir = std::fs::File::open(&path).unwrap();
    let uid = crate::syscall::geteuid();
    let evidence: std::collections::BTreeMap<u32, AttemptEvidenceV1> = (0..8u32)
        .map(|slot| {
            let mut record = session(&unit().definition_sha256);
            let target = attempt_process(slot);
            record.process = SessionProcess {
                pid: target.pid,
                start_ticks: target.start_ticks,
                uid,
            };
            let item = AttemptEvidenceV1 {
                schema_version: 1,
                slot_index: slot,
                average_fps: 1.0,
                native_completion: None,
                long_session: Some(Box::new(record)),
                thermal: None,
            };
            (slot, item)
        })
        .collect();
    let windows = |intervals: u32| -> Vec<AttemptResident> {
        let mut observer = ResidentObserver::default();
        for slot in 0..8u32 {
            let target = attempt_process(slot);
            observer.arm(target, Some(slot), 5_010);
            let mut histogram = LatencyHistogram::new();
            for _ in 0..50 {
                histogram.record(700_000);
            }
            for _ in 0..intervals {
                observer.observe_intervals(
                    &[IntervalRecord {
                        process_pid: Some(target.pid.as_u32()),
                        class: TaskClass::Game,
                        samples: 50,
                        max_ns: 700_000,
                        histogram: histogram.snapshot(),
                        ..Default::default()
                    }],
                    0,
                );
                observer.observe_gpu_sample();
            }
            observer.end_attempt(6_190);
        }
        // These pids are not live processes, so the kernel shows no cgroup for them: the window carries what a real
        // process's /proc/<pid>/cgroup would have shown.
        let mut windows = observer.freeze(6_190);
        for window in &mut windows {
            window.cgroup = "/app.slice/game.scope".to_owned();
        }
        windows
    };
    let receipts = |bound_all: bool| {
        let items: Vec<_> = (0..8u32)
            .map(|slot| {
                let target = attempt_process(slot);
                serde_json::json!({
                    "attempt": slot, "pid": target.pid, "start_ticks": target.start_ticks, "uid": uid,
                    "source_bound_to_game": bound_all || slot != 3, "source_healthy": true, "snapshots": 5,
                })
            })
            .collect();
        let file = path.join("telemetry-receipts.json");
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        std::fs::write(&file, serde_json::to_vec(&items).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    let sealed = path.join(resident_observation::FILE);
    // One attempt's renderer was not bound to the game: no record.
    receipts(false);
    runner
        .seal_resident_observation(&dir, &windows(12), &evidence)
        .unwrap();
    assert!(!sealed.exists());
    // Too few samples to say the game was watched: no record.
    receipts(true);
    runner
        .seal_resident_observation(&dir, &windows(3), &evidence)
        .unwrap();
    assert!(!sealed.exists());
    // A window missing for an attempt: no record.
    let mut missing = windows(12);
    missing.pop();
    runner
        .seal_resident_observation(&dir, &missing, &evidence)
        .unwrap();
    assert!(!sealed.exists());
    // Everything observed: the record exists and the independent verifier accepts it.
    runner
        .seal_resident_observation(&dir, &windows(12), &evidence)
        .unwrap();
    let processes = crate::autotune::research::child_verify_rules::processes_of(&evidence);
    resident_observation::verify(&std::fs::read(&sealed).unwrap(), &processes).unwrap();
}

/// The whole measurement chain from the runtime's own events: per-attempt populations, frames, GPU samples and CPU
/// frequency events fed to the observer produce, at study end, the paired runnable-latency record G5 stands on,
/// with the guard facts read from the production objective-signal builder and the production decision applied by the
/// verifier. The action that shortened the tail in every pair decides Keep; with a guard fact missing there is no record.
#[test]
fn the_latency_record_is_derived_from_the_runtimes_own_events_and_decides_through_the_production_objective()
 {
    use crate::{
        autotune::research::{
            action_latency::{self, Decision},
            child_verify_rules::processes_of,
            resident_observer::SharedResident,
            treatment::{Treatment, treatments_of},
        },
        metrics::{IntervalRecord, LatencyHistogram},
        process_tree::TaskClass,
        recorder::{CpuFreqRecord, FrameEvent, GpuSample},
    };
    let run = |with_cpu_freq: bool| {
        let (root, runner, study, gate, _parent) = build(&unit().definition_sha256);
        // The fixture sessions sit at synthetic unix times: the observers stamp their epochs from a clock that walks them.
        crate::autotune::research::resident_observer::TEST_CLOCK
            .with(|clock| clock.set(Some(super::latency_fixtures::clock())));
        let resident: SharedResident = Default::default();
        let mut runner = runner.with_resident(resident.clone());
        let path = root.path().join("studies").join(&study.study_sha256[..16]);
        let mut acted = std::collections::BTreeSet::new();
        let mut schedule: Option<Vec<Treatment>> = None;
        let mut elapsed = 0u64;
        let mut outcome = None;
        for _ in 0..20_000 {
            if let Some(done) = runner.poll(&study, 100).unwrap() {
                outcome = Some(done);
                break;
            }
            if schedule.is_none() && path.join("intent.json").is_file() {
                let intent: ResearchIntentV1 =
                    serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap())
                        .unwrap();
                schedule = treatments_of(&intent).unwrap();
            }
            // The runtime's events concern the process the observer is armed on (the attested one).
            let (Some(process), Some(schedule)) =
                (resident.lock().unwrap().target(), schedule.as_ref())
            else {
                continue;
            };
            // The normal controller applies the action once in every attempt the gate allows, and restores it.
            if acted.insert(process.pid) {
                act_once(&gate, process);
            }
            // The action shortens the runnable tail from 20 ms to 1.5 ms in every attempt treated On.
            let on = schedule[(process.pid.as_u32() - 5_000) as usize] == Treatment::On;
            let tail = if on { 1_500_000 } else { 20_000_000 };
            let mut histogram = LatencyHistogram::new();
            for _ in 0..450 {
                histogram.record(500_000);
            }
            for _ in 0..50 {
                histogram.record(tail);
            }
            let mut guard = resident.lock().unwrap();
            for _ in 0..4 {
                elapsed += 1_000;
                guard.observe_intervals(
                    &[IntervalRecord {
                        elapsed_ms: elapsed,
                        process_pid: Some(process.pid.as_u32()),
                        class: TaskClass::Game,
                        samples: 500,
                        max_ns: tail,
                        histogram: histogram.snapshot(),
                        ..Default::default()
                    }],
                    0,
                );
                for frame in 0..100 {
                    guard.observe_frame(&FrameEvent {
                        elapsed_ms: elapsed,
                        frametime_ms: 12.0 + f64::from(frame % 3),
                    });
                }
                guard.observe_gpu(&GpuSample {
                    elapsed_ms: elapsed,
                    gpu_busy_percent: Some(60),
                    gpu_clock_mhz: Some(2_000),
                    temp_millidegrees: Some(60_000),
                    ..Default::default()
                });
                if with_cpu_freq {
                    guard.observe_cpu_freq(&CpuFreqRecord {
                        elapsed_ms: elapsed,
                        cpu: 0,
                        freq_khz: 4_000_000,
                        timestamp_ns: 0,
                    });
                }
            }
        }
        let outcome = outcome.expect("the study ended");
        (root, study, path, outcome)
    };
    let (_root, _study, path, outcome) = run(true);
    assert_eq!(outcome.completed, 8);
    let record = path.join(action_latency::FILE);
    assert!(
        record.is_file(),
        "every planned pair has its guarded population"
    );
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
    let evidence: std::collections::BTreeMap<
        u32,
        crate::autotune::research::report::AttemptEvidenceV1,
    > = std::fs::read_dir(path.join("evidence"))
        .unwrap()
        .flatten()
        .map(|entry| {
            let item: crate::autotune::research::report::AttemptEvidenceV1 =
                serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            (item.slot_index, item)
        })
        .collect();
    let result = action_latency::verify(
        &std::fs::read(&record).unwrap(),
        &treatments_of(&intent).unwrap().unwrap(),
        &processes_of(&evidence),
    )
    .unwrap();
    assert_eq!((result.pairs, result.decision), (4, Decision::Keep));
    // Without the CPU frequency events the power guard was never observed: no guard facts, no record.
    let (_root, _study, path, _outcome) = run(false);
    assert!(!path.join(action_latency::FILE).exists());
}

/// The runtime's own archivers produce the controller's and the worker's accounts: the controller's journal writes
/// (through the real durable writer) are tapped, the worker's audit file is read, and at study end both are archived
/// beside the study, so the study qualifies and replays without any hand-written copy. Without a worker audit that
/// shows the action, the same study does not qualify.
#[test]
fn the_runtimes_archivers_make_the_controllers_and_the_workers_accounts_that_the_replay_joins() {
    use std::io::Write;

    use crate::autotune::{
        controller_journal::write_controller_journal_record,
        research::evidence_archive::{JournalArchive, SharedJournalArchive, install_journal_tap},
    };
    let _turn = crate::autotune::research::evidence_archive::TAP_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let run = |worker_shows_the_action: bool| {
        let (root, runner, study, gate, _parent) = build(&unit().definition_sha256);
        let archive: SharedJournalArchive =
            std::sync::Arc::new(std::sync::Mutex::new(JournalArchive::default()));
        install_journal_tap(&archive);
        let scratch = tempfile::tempdir().unwrap();
        let (journal_path, audit_path) = (
            scratch.path().join("controller_journal.json"),
            scratch.path().join("actions.jsonl"),
        );
        std::fs::File::create(&audit_path).unwrap();
        let mut runner = runner.with_provenance(archive, audit_path.clone());
        let (journal_for_hook, audit_for_hook) = (journal_path.clone(), audit_path.clone());
        // The worker's export is one hash chain across every transaction it writes.
        let export = std::sync::Arc::new(std::sync::Mutex::new(
            crate::autotune::research::evidence_producers::WorkerExport::default(),
        ));
        ON_ACT.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |target, at| {
                // The controller walks the transaction to reverted through its real durable journal writer.
                let record = crate::autotune::research::evidence_producers::controller_journal_record(
                    &format!("exp-{:?}", target.pid),
                    "cpu-affinity-profile:game-main",
                    (target.pid.as_u32(), target.start_ticks),
                    (at, at + 1),
                );
                let record: crate::autotune::controller_journal::ControllerJournalRecord =
                    serde_json::from_value(record).unwrap_or_else(|_| {
                        let mut value = crate::autotune::research::evidence_producers::controller_journal_record(
                            "x", "cpu-affinity-profile:game-main", (1, 1), (1, 2));
                        value["boot_id"] = serde_json::json!(stutter_core::boot::current_boot_id().unwrap());
                        serde_json::from_value(value).unwrap()
                    });
                let mut value = serde_json::to_value(&record).unwrap();
                value["boot_id"] = serde_json::json!(stutter_core::boot::current_boot_id().unwrap());
                write_controller_journal_record(&journal_for_hook, &serde_json::from_value(value).unwrap()).unwrap();
                if worker_shows_the_action {
                    // The worker appends the lifecycle of the transaction it issued to its research export.
                    let mut export = export.lock().unwrap();
                    let before = export.records().len();
                    export.transaction(
                        &crate::autotune::research::evidence_producers::worker_transaction_of(&format!("exp-{:?}", target.pid)),
                        "cpu-affinity-profile:game-main",
                        (target.pid.as_u32(), target.start_ticks),
                        (at, at + 1, at + 1),
                    );
                    let mut file = std::fs::OpenOptions::new().append(true).open(&audit_for_hook).unwrap();
                    for record in &export.records()[before..] {
                        writeln!(file, "{record}").unwrap();
                    }
                }
            }));
        });
        FIXED_SESSIONS.set(true);
        let outcome = run_acting(&mut runner, &study, &gate);
        ON_ACT.with(|hook| *hook.borrow_mut() = None);
        crate::autotune::controller_journal::set_journal_tap(None);
        let path = root.path().join("studies").join(&study.study_sha256[..16]);
        (outcome, path, root)
    };
    // The archivers wrote both accounts, and the replay (the same function the live outcome uses) joins them.
    let (outcome, path, _root) = run(true);
    assert_eq!(outcome.stage_evidence, Some(true));
    for file in ["controller-journal.json", "worker-transactions.json"] {
        assert!(path.join(file).is_file(), "{file} was archived");
    }
    let journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("controller-journal.json")).unwrap())
            .unwrap();
    assert_eq!(
        journal.as_array().unwrap().len(),
        4,
        "one final record per treated attempt"
    );
    // A worker audit that never shows the action leaves the study without its corroboration.
    let (outcome, _path, _root) = run(false);
    assert_eq!(outcome.stage_evidence, Some(false));
}

/// The abstention's planner observation comes from the runtime's own decision stream: with the planner evaluating the
/// game process and never offering the action, the study is a justified abstention with no hand-written file; with the
/// action offered but never applied, or with the planner never watching, it is not.
#[test]
fn a_justified_abstention_stands_on_the_planner_observation_the_runtime_derived_from_its_decisions()
{
    use crate::autotune::research::resident_observer::SharedResident;
    let run = |offer: Option<&'static str>, watch: bool| {
        let (root, runner, study, _gate, _parent) = build(&unit().definition_sha256);
        // The fixture sessions sit at synthetic unix times: the observers stamp their epochs from a clock that walks them.
        crate::autotune::research::resident_observer::TEST_CLOCK
            .with(|clock| clock.set(Some(super::latency_fixtures::clock())));
        let resident: SharedResident = Default::default();
        let mut runner = runner.with_resident(resident.clone());
        let outcome = 'run: {
            for _ in 0..20_000 {
                if let Some(outcome) = runner.poll(&study, 100).unwrap() {
                    break 'run outcome;
                }
                let mut guard = resident.lock().unwrap();
                if let (true, Some(target)) = (watch, guard.target()) {
                    let planner = offer
                        .map(crate::autotune::research::resident_observer::tests::offering)
                        .unwrap_or_default();
                    // The scheduler events the runtime also handles for the game, so the window is a direct observation.
                    let mut histogram = crate::metrics::LatencyHistogram::new();
                    for _ in 0..10 {
                        histogram.record(500_000);
                    }
                    guard.observe_intervals(
                        &[crate::metrics::IntervalRecord {
                            process_pid: Some(target.pid.as_u32()),
                            class: crate::process_tree::TaskClass::Game,
                            samples: 10,
                            max_ns: 500_000,
                            histogram: histogram.snapshot(),
                            ..Default::default()
                        }],
                        0,
                    );
                    for _ in 0..20 {
                        guard.observe_decision(Some(target.pid.as_u32()), Some(&planner));
                    }
                }
            }
            panic!("the study never ended");
        };
        (outcome, root)
    };
    let (outcome, _root) = run(None, true);
    assert!(outcome.action_not_indicated);
    let (offered, _root) = run(Some("cpu_affinity_profile"), true);
    assert!(
        !offered.action_not_indicated,
        "the planner offered the action"
    );
    let (unwatched, _root) = run(None, false);
    assert!(
        !unwatched.action_not_indicated,
        "the planner was never seen evaluating the game"
    );
}
