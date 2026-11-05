//! Service initialization and read-only state mirroring around the shared soak loop.
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use anyhow::{Context, ensure};
use stutter_validation_runner::model::BoundedServiceAction;

use super::{
    plan::Operations,
    records::{Profile, SCHEMA_VERSION, SoakState, read_state, unix_ms, write_state},
    service::Binding,
    service_presence::Presence,
    world::World,
};
use crate::{
    service_campaign::{
        manifest::Manifest,
        model::{Stage, State},
        runner,
    },
    validation_campaign::state::write_atomic,
};

pub(crate) fn init(state: &State, dir: &Path) -> anyhow::Result<()> {
    runner::integrity(&state.repository, state)?;
    if dir.join("soak.json").exists() {
        let existing = read_state(dir)?;
        ensure!(
            existing.profile == Profile::Service
                && existing.source_commit == state.source_commit
                && existing.stutter_sha256 == state.stutter_sha256
                && existing.xtask_sha256 == state.xtask_sha256,
            "existing service soak has a different source binding"
        );
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let manifest = Manifest::load_for(
        &state.repository.join(state.manifest_file()),
        state.acceptance,
    )?;
    write_state(
        dir,
        &SoakState {
            schema_version: SCHEMA_VERSION,
            id: format!("{}-soak", state.campaign_id),
            profile: Profile::Service,
            seed: state.created_unix_ms,
            hours: manifest.coverage.minimum_dwell_seconds as f64 / 3600.0,
            stutter_bin: state.pinned_stutter(),
            stutter_sha256: state.stutter_sha256.clone(),
            xtask_sha256: state.xtask_sha256.clone(),
            source_commit: state.source_commit.clone(),
            allow_dirty: false,
            created_unix_ms: unix_ms(),
            started_unix_ms: None,
            cpus: Some(super::pick_cpus()?),
            next_round: 0,
            finished: false,
            service: Some(Binding::new(state, Stage::Soak)?),
        },
    )
}

pub(crate) fn campaign_run(state: &State, dir: &Path) -> anyhow::Result<super::verify::Report> {
    init(state, dir)?;
    super::run_soak(&state.repository, dir)?;
    let report = super::verify::verify(dir)?;
    write_atomic(
        &dir.join("verify.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

pub(super) fn run(root: &Path, dir: &Path, state: &mut SoakState) -> anyhow::Result<()> {
    let mut binding = state
        .service
        .clone()
        .context("missing pinned service binding")?;
    runner::integrity(root, &binding.campaign)?;
    ensure!(
        state.source_commit == binding.campaign.source_commit
            && state.stutter_sha256 == binding.campaign.stutter_sha256
            && state.xtask_sha256 == binding.campaign.xtask_sha256
            && !state.allow_dirty,
        "service soak provenance differs"
    );
    let dir = dir.canonicalize()?;
    let mut world = World::new(
        root,
        &dir,
        &state.stutter_bin,
        state.cpus.context("missing service CPUs")?,
    )?;
    let presence = Arc::new(Presence::start(&dir)?);
    binding.presence = Some(presence.clone());
    world.bind_service(binding.clone())?;
    capture_offsets(&dir, &world)?;
    super::install_signal_handlers();
    let result = (|| {
        world.prepare_round(Operations::Both)?;
        binding.start()?;
        super::drive_soak(&dir, state, &mut world)
    })();
    // Every path stops the owned service, preserves evidence, and joins the dwell collector.
    let stopped = settle(&dir, &mut world);
    let collected = presence.finish();
    result?;
    stopped?;
    collected
}

pub(super) fn manifest(state: &SoakState) -> anyhow::Result<Manifest> {
    let binding = state.service.as_ref().context("service binding missing")?;
    let path = binding
        .campaign
        .repository
        .join(binding.campaign.manifest_file());
    ensure!(
        super::sha256(&path)? == binding.campaign.manifest_sha256,
        "service manifest changed"
    );
    let manifest = Manifest::load_for(&path, binding.campaign.acceptance)?;
    ensure!(
        state.hours == manifest.coverage.minimum_dwell_seconds as f64 / 3600.0,
        "service dwell differs from its frozen manifest"
    );
    Ok(manifest)
}

pub(super) fn minimums(state: &SoakState) -> anyhow::Result<super::coverage::Minimums> {
    if state.profile != Profile::Service {
        return Ok(super::coverage::Minimums::for_profile(state.profile));
    }
    let coverage = manifest(state)?.coverage;
    if state
        .service
        .as_ref()
        .is_some_and(|binding| binding.campaign.rehearsal)
    {
        return Ok(super::coverage::Minimums::rehearsal());
    }
    let acceptance = state
        .service
        .as_ref()
        .is_some_and(|binding| binding.campaign.acceptance);
    let mut floor = if acceptance {
        super::coverage::Minimums::acceptance()
    } else {
        super::coverage::Minimums::FULL
    };
    if acceptance {
        floor.clean_rounds =
            super::plan::SERVICE_CYCLE_LEN * coverage.minimum_clean_cycles as usize;
    }
    floor.hours = coverage.minimum_dwell_seconds as f64 / 3600.0;
    floor.clean_cycles = coverage.minimum_clean_cycles as usize;
    floor.per_kind = coverage.minimum_dangerous_event_repetitions as usize;
    floor.service_events = true;
    floor.per_operation_kept = coverage.minimum_keeps_per_operation as usize;
    floor.per_operation_reverted = coverage.minimum_reverts_per_operation as usize;
    Ok(floor)
}

fn sources(world: &World) -> BTreeMap<&'static str, std::path::PathBuf> {
    BTreeMap::from([
        ("autotune/history.jsonl", world.history_path()),
        ("audit/actions.jsonl", world.audit_path()),
    ])
}

fn capture_offsets(dir: &Path, world: &World) -> anyhow::Result<()> {
    let path = dir.join("service-history-offsets.json");
    if path.exists() {
        return Ok(());
    }
    let offsets: BTreeMap<_, _> = sources(world)
        .into_iter()
        .map(|(name, path)| {
            (
                name,
                fs::metadata(path).map_or(0, |metadata| metadata.len()),
            )
        })
        .collect();
    write_atomic(&path, &serde_json::to_vec_pretty(&offsets)?)
}

pub(super) fn sync(dir: &Path, world: &World) -> anyhow::Result<()> {
    if world.service.is_none() {
        return Ok(());
    }
    let offsets: BTreeMap<String, u64> =
        serde_json::from_slice(&fs::read(dir.join("service-history-offsets.json"))?)?;
    let target = dir.join("home/.local/state/stutter");
    for (name, path) in sources(world) {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(error) => return Err(error.into()),
        };
        let offset = usize::try_from(
            *offsets
                .get(name)
                .context("service history offset missing")?,
        )?;
        ensure!(
            offset <= bytes.len(),
            "service history was truncated or replaced during the soak"
        );
        let slice = &bytes[offset..];
        let complete = slice
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let output = target.join(name);
        fs::create_dir_all(output.parent().context("mirror parent missing")?)?;
        write_atomic(&output, &slice[..complete])?;
    }
    for (name, source) in [
        ("controller_journal.json", world.journal_path()),
        ("kept-actions.json", world.ledger_path()),
    ] {
        let output = target.join("autotune").join(name);
        match fs::read(source) {
            Ok(bytes) => write_atomic(&output, &bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if output.exists() {
                    fs::remove_file(output)?;
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn settle(dir: &Path, world: &mut World) -> anyhow::Result<()> {
    if let Some(binding) = &world.service {
        let report = binding.action(BoundedServiceAction::Stop)?;
        write_atomic(
            &dir.join("final-service-runtime.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        sync(dir, world)?;
    } else {
        world.kill_worker()?;
    }
    Ok(())
}

pub(super) fn runtime_checks(dir: &Path) -> anyhow::Result<Vec<super::verify::Check>> {
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("final-service-runtime.json"))?)?;
    let idle = |actor: &str| {
        report[actor].as_str().is_some_and(|properties| {
            properties.lines().any(|line| line == "MainPID=0")
                && properties
                    .lines()
                    .any(|line| line == "ActiveState=inactive")
        })
    };
    let mut checks = vec![super::verify::Check {
        name: "packaged_service_final_runtime".into(),
        passed: idle("controller")
            && idle("worker")
            && report["ownership_verified"] == true
            && report["worker_tokens"]["readable"] == true
            && report["worker_tokens"]["outstanding"] == 0,
        detail:
            "fixed service actors stopped; receipt ownership intact; root-issued tokens settled"
                .into(),
    }];
    let state = read_state(dir)?;
    let (repetitions, required) = event_and_reboot_floors(&state)?;
    let rounds = super::records::read_rounds(dir)?;
    for event in super::plan::SERVICE_EVENT_KINDS {
        let count = rounds
            .iter()
            .filter(|round| round.outcome == "ok" && round.violations.is_empty())
            .flat_map(|round| &round.events)
            .filter(|record| record.kind == event)
            .count();
        checks.push(super::verify::Check {
            name: format!("service_repeated_{event}"),
            passed: count >= repetitions,
            detail: format!("{count} clean audited executions; at least {repetitions} required"),
        });
    }
    // Reboot receipts come only from the typed reboot stage. The current boot ID or historical
    // direct-controller evidence cannot stand in for them.
    let (good, detail) = match dir.parent() {
        Some(campaign) => super::service_reboots::verified_receipts(
            campaign,
            &state
                .service
                .as_ref()
                .map(|binding| binding.campaign.boot_ids.clone())
                .unwrap_or_default(),
            required,
        ),
        None => (0, "the soak has no campaign directory".to_owned()),
    };
    checks.push(super::verify::Check {
        name: "service_real_reboots".into(),
        passed: good >= required,
        detail,
    });
    checks.push(runtime_bound_check(dir, &state, &rounds)?);
    Ok(checks)
}

/// The whole soak, idle dwell included, ran inside the frozen maximum. The span ends at the later
/// of the last round and the last independently sampled presence interval, so it is not just the
/// rounds' own duration.
fn runtime_bound_check(
    dir: &Path,
    state: &SoakState,
    rounds: &[super::records::RoundRecord],
) -> anyhow::Result<super::verify::Check> {
    let maximum_ms = manifest(state)?.coverage.maximum_dwell_seconds * 1000;
    let last_round = rounds
        .iter()
        .map(|round| round.finished_unix_ms)
        .max()
        .unwrap_or(0);
    let last_presence = std::fs::read_to_string(dir.join("presence.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|row| row["end_unix_ms"].as_u64())
        .max()
        .unwrap_or(0);
    let started = state.started_unix_ms.unwrap_or(0);
    let span_ms = last_round.max(last_presence).saturating_sub(started);
    Ok(super::verify::Check {
        name: "service_soak_within_maximum_runtime".into(),
        passed: started > 0 && span_ms <= maximum_ms,
        detail: format!(
            "the soak spanned {:.2} h from its first round to its last presence sample; at most {:.2} h allowed",
            span_ms as f64 / 3_600_000.0,
            maximum_ms as f64 / 3_600_000.0
        ),
    })
}

/// After every round, what the schedule projects to: informational evidence, never a gate. Six
/// clean 22-round cycles and the verified-live dwell are two different requirements; the run needs
/// whichever takes longer, inside the frozen maximum. Before one full cycle the figure is
/// preliminary because the service-only rounds have not been seen yet.
pub(super) fn record_projection(
    dir: &Path,
    state: &SoakState,
    index: u32,
    started_unix_ms: u64,
) -> anyhow::Result<bool> {
    let manifest = manifest(state)?;
    let binding = state.service.as_ref().context("service binding missing")?;
    let rounds_done = u64::from(index) + 1;
    let elapsed_ms = unix_ms().saturating_sub(started_unix_ms).max(1);
    let verified_ms = super::service_presence::verified_ms(dir, &binding.campaign)?;
    let cycle_len = super::plan::SERVICE_CYCLE_LEN as u64;
    let hours = |ms: f64| ms / 3_600_000.0;
    let duty = verified_ms as f64 / elapsed_ms as f64;
    // Attempts that injected nothing were repeated: their time is already spent and will not
    // recur at every remaining round, so it is carried as a fixed overhead, not averaged in.
    let overhead_ms: u64 = super::records::read_rounds(dir)?
        .iter()
        .filter(|round| round.outcome == super::round::NOT_ATTEMPTED)
        .map(|round| round.finished_unix_ms.saturating_sub(round.started_unix_ms))
        .sum();
    let total_rounds = cycle_len * u64::from(manifest.coverage.minimum_clean_cycles);
    let per_round_ms = elapsed_ms.saturating_sub(overhead_ms) as f64 / rounds_done as f64;
    let for_cycles_h = hours(project_cycles_ms(
        elapsed_ms,
        overhead_ms,
        rounds_done,
        total_rounds,
    ));
    let minimum_dwell_h = manifest.coverage.minimum_dwell_seconds as f64 / 3600.0;
    let maximum_dwell_h = manifest.coverage.maximum_dwell_seconds as f64 / 3600.0;
    let for_dwell_h = wall_for_dwell(for_cycles_h, duty, minimum_dwell_h);
    let required_h = for_cycles_h.max(for_dwell_h);
    let value = serde_json::json!({
        "preliminary": rounds_done < cycle_len,
        "rounds_done": rounds_done,
        "complete_cycles": rounds_done / cycle_len,
        "elapsed_wall_h": hours(elapsed_ms as f64),
        "verified_live_h": hours(verified_ms as f64),
        "service_presence_duty_ratio": duty,
        "seconds_per_round": per_round_ms / 1000.0,
        "projected_wall_h_for_required_cycles": for_cycles_h,
        "projected_wall_h_for_verified_dwell": for_dwell_h,
        "required_projected_wall_h": required_h,
        "minimum_verified_dwell_h": minimum_dwell_h,
        "maximum_dwell_h": maximum_dwell_h,
        "margin_h": maximum_dwell_h - required_h,
        "fits_inside_the_cap": required_h <= maximum_dwell_h,
    });
    write_atomic(
        &dir.join("timing-projection.json"),
        &serde_json::to_vec_pretty(&value)?,
    )?;
    // Only a full cycle makes the figure meaningful; before that it is preliminary.
    Ok(rounds_done < cycle_len || required_h <= maximum_dwell_h)
}

/// Wall time to finish `total_rounds` scheduled rounds: the pace of the rounds done so far, with
/// the time lost to repeated attempts counted once.
fn project_cycles_ms(
    elapsed_ms: u64,
    overhead_ms: u64,
    rounds_done: u64,
    total_rounds: u64,
) -> f64 {
    let pace = elapsed_ms.saturating_sub(overhead_ms) as f64 / rounds_done.max(1) as f64;
    pace * total_rounds as f64 + overhead_ms as f64
}

/// Wall time to a verified dwell of `dwell_h` after the cycles: the cycles earn `duty` of their
/// wall time, and the coverage-complete idle phase earns the rest at full presence.
fn wall_for_dwell(cycles_h: f64, duty: f64, dwell_h: f64) -> f64 {
    if duty <= 0.0 {
        return f64::INFINITY;
    }
    cycles_h + (dwell_h - duty * cycles_h).max(0.0)
}

/// What the service runtime checks require of this campaign: clean executions of each service
/// event and verified real reboots. A rehearsal proves less by its own explicit contract; a
/// counted campaign must match its frozen manifest exactly, so a counted campaign can never
/// inherit a rehearsal's weaker gates through a mutated state field.
fn event_and_reboot_floors(state: &SoakState) -> anyhow::Result<(usize, usize)> {
    let binding = state.service.as_ref().context("service binding missing")?;
    let coverage = manifest(state)?.coverage;
    floors(
        binding.campaign.rehearsal,
        binding.campaign.required_reboots,
        coverage.minimum_dangerous_event_repetitions,
        coverage.minimum_real_reboots,
    )
}

fn floors(
    rehearsal: bool,
    required_reboots: u32,
    manifest_repetitions: u32,
    manifest_reboots: u32,
) -> anyhow::Result<(usize, usize)> {
    if rehearsal {
        ensure!(
            required_reboots == 0,
            "a rehearsal records no real reboots, but requires {required_reboots}"
        );
        return Ok((super::coverage::Minimums::rehearsal().per_kind, 0));
    }
    ensure!(
        required_reboots == manifest_reboots,
        "the campaign requires {required_reboots} reboots but its frozen manifest requires {manifest_reboots}"
    );
    Ok((manifest_repetitions as usize, manifest_reboots as usize))
}

#[cfg(test)]
mod floor_tests {
    use super::floors;

    #[test]
    fn a_rehearsal_needs_one_execution_and_no_reboot() {
        assert_eq!(floors(true, 0, 6, 2).unwrap(), (1, 0));
    }

    #[test]
    fn a_counted_campaign_keeps_the_frozen_manifest_floors() {
        assert_eq!(floors(false, 2, 6, 2).unwrap(), (6, 2));
    }

    #[test]
    fn a_counted_campaign_cannot_inherit_a_rehearsals_weaker_gates() {
        assert!(floors(false, 0, 6, 2).is_err());
        assert!(floors(false, 1, 6, 2).is_err());
    }

    #[test]
    fn a_rehearsal_that_demands_reboots_is_inconsistent() {
        assert!(floors(true, 2, 6, 2).is_err());
    }
}

#[cfg(test)]
mod projection_tests {
    use super::{project_cycles_ms, wall_for_dwell};

    #[test]
    fn a_repeated_attempt_is_counted_once_not_at_every_remaining_round() {
        let (round, total) = (100_000_u64, 132_u64);
        let clean = project_cycles_ms(22 * round, 0, 22, total);
        // One refused attempt of 150 s in the first cycle adds 150 s, not 150 s per round.
        let with_retry = project_cycles_ms(22 * round + 150_000, 150_000, 22, total);
        assert!((with_retry - clean - 150_000.0).abs() < 1.0);
        // Many repeated attempts still only add their own time.
        let many = project_cycles_ms(22 * round + 450_000, 450_000, 22, total);
        assert!((many - clean - 450_000.0).abs() < 1.0);
        // No rounds done yet cannot divide by zero.
        assert!(project_cycles_ms(10, 0, 0, total).is_finite());
    }

    #[test]
    fn the_idle_phase_tops_up_only_the_missing_dwell() {
        // Cycles of 9 h at 80 % presence earn 7.2 h; the missing 0.8 h is idle presence.
        assert!((wall_for_dwell(9.0, 0.8, 8.0) - 9.8).abs() < 1e-9);
        // Cycles that already earn the dwell need no top-up.
        assert!((wall_for_dwell(9.0, 0.95, 8.0) - 9.0).abs() < 1e-9);
        assert!(wall_for_dwell(9.0, 0.0, 8.0).is_infinite());
    }
}
