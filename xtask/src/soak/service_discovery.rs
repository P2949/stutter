//! Live auto-focus acceptance through the installed service, never a fake resolver input.
use std::{collections::BTreeSet, fs, path::Path, time::Duration};

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter_validation_runner::model::BoundedServiceAction as Action;

use super::{
    history::Tail,
    plan::{Operations, timing},
    records::Profile,
    service::Binding,
    service_probe::{Probe, focus_matches, stop_game},
    world::World,
};
use crate::service_campaign::model::{Stage, State};

fn live_group(pid: u32) -> Option<Value> {
    let mut cache = stutter::api::focus::FocusCache::default();
    stutter::api::focus::focus_snapshot_at(Path::new("/proc"), &mut cache, 0, None);
    std::thread::sleep(Duration::from_secs(1));
    let snapshot =
        stutter::api::focus::focus_snapshot_at(Path::new("/proc"), &mut cache, 1000, None);
    let group = snapshot
        .groups
        .iter()
        .find(|group| group.member_pids.contains(&pid))?;
    // Keep only decision evidence; omit unrelated processes and command lines.
    Some(
        json!({"kind":group.kind,"confidence":group.confidence,"score":group.score,"root_pids":group.root_pids}),
    )
}

pub(crate) fn run(state: &State, dir: &Path) -> anyhow::Result<Value> {
    fs::create_dir_all(dir)?;
    let cpus = super::pick_cpus()?;
    let binding = Binding::new(state, Stage::Discovery)?;
    let mut world = World::new(&state.repository, dir, &state.pinned_stutter(), cpus)?;
    world.bind_service(binding.clone())?;
    world.prepare_round(Operations::Both)?;
    let mut probe = Probe {
        tail: Tail::at_end(&world.history_path()),
        records: vec![],
        owned: BTreeSet::new(),
    };
    let mut controller =
        world.spawn_controller(dir, "discovery", 0, Operations::Both, timing(Profile::Full))?;
    let original_pid = controller.child.pid();
    let result = (|| -> anyhow::Result<Value> {
        let mut cases = Vec::new();
        cases.push(json!({"scenario":"idle_no_supported_target", "observations":probe.quiet(&mut controller, 6, None)?}));
        // These are the actual same pinned workload, discovered from its real executable identity.
        // No class, confidence, PID or resolver snapshot is supplied to the controller.
        let game_binary = world.game.clone();
        for (name, path) in [
            ("browser", dir.join("browser/firefox")),
            ("desktop_compositor", dir.join("desktop/kwin_wayland")),
            ("unknown", dir.join("unknown/unclassified-workload")),
            (
                "low_confidence",
                dir.join("low-confidence/steamapps/common/WeakGame/WeakGame"),
            ),
        ] {
            fs::create_dir_all(path.parent().context("fixture parent missing")?)?;
            fs::copy(&world.xtask, &path)?;
            world.game = path;
            let mut fixture = world.spawn_game(dir, name, 30)?;
            probe.own(fixture.child.pid());
            let _pressure = world.spawn_burst(cpus.0, 8, 30)?;
            let diagnostic = live_group(fixture.child.pid());
            if name == "low_confidence" {
                let group = diagnostic
                    .as_ref()
                    .context("low-confidence fixture produced no real focus group")?;
                ensure!(
                    group["kind"] == "Game"
                        && group["confidence"]
                            .as_f64()
                            .is_some_and(|confidence| confidence < 0.85),
                    "fixture did not exercise an actual low-confidence game group: {group}"
                );
            }
            let forbidden =
                matches!(name, "unknown" | "low_confidence").then_some(fixture.child.pid());
            let rows = probe.quiet(&mut controller, 8, forbidden)?;
            cases.push(json!({"scenario":name,"pid":fixture.child.pid(),"starttime_ticks":fixture.ticks,"diagnostic_group":diagnostic,"observations":rows}));
            stop_game(&mut fixture)?;
        }
        world.game = game_binary;
        let mut first = world.spawn_game(dir, "first-game", 180)?;
        probe.own(first.child.pid());
        probe.wait(&mut controller, 30, |row| focus_matches(row, &first))?;
        cases.push(json!({"scenario":"single_game_stabilizes","pid":first.child.pid(),"starttime_ticks":first.ticks}));
        let second_binary = dir
            .join("second-game-install/steamapps/common/SecondAcceptanceGame/SecondAcceptanceGame");
        fs::create_dir_all(
            second_binary
                .parent()
                .context("second game parent missing")?,
        )?;
        fs::copy(&world.xtask, &second_binary)?;
        let first_binary = std::mem::replace(&mut world.game, second_binary);
        let mut second = world.spawn_game(dir, "second-game", 60)?;
        probe.own(second.child.pid());
        world.game = first_binary;
        let groups =
            json!({"first":live_group(first.child.pid()),"second":live_group(second.child.pid())});
        fs::write(
            dir.join("two-game-groups.json"),
            serde_json::to_vec_pretty(&groups)?,
        )?;
        probe.wait(&mut controller, 15, |row| {
            row["target"].is_null()
                && row["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("ambiguous_game_roots"))
        })?;
        cases.push(json!({"scenario":"two_plausible_games","observations":probe.quiet(&mut controller, 6, None)?}));
        stop_game(&mut second)?;
        stop_game(&mut first)?;
        // Each replacement is a real new process. Numeric-PID reuse has an additional source test
        // with distinct start ticks; this live case proves replacement through the actual service.
        for n in 0..6 {
            let mut game = world.spawn_transient_game(dir, &format!("oscillating-{n}"), 10)?;
            probe.own(game.child.pid());
            cases.push(json!({"scenario":"rapid_focus_oscillation","pid":game.child.pid(),"starttime_ticks":game.ticks,"observations":probe.quiet(&mut controller, 1, Some(game.child.pid()))?}));
            stop_game(&mut game)?;
        }
        let mut replacement = world.spawn_game(dir, "replacement-game", 590)?;
        probe.own(replacement.child.pid());
        ensure!(
            replacement.child.pid() != first.child.pid() || replacement.ticks != first.ticks,
            "replacement reused an identical process identity"
        );
        probe.wait(&mut controller, 30, |row| focus_matches(row, &replacement))?;
        // An experiment cut short by the two-game phase leaves its action in the controller's
        // five-minute candidate cooldown; the replacement may only start once that has passed.
        let _pressure = world.spawn_burst(cpus.0, 8, 590)?;
        // The controller keeps only a measured improvement, so an experiment can honestly end
        // inconclusive and be reverted. The scenario asks for a supported keep, so it allows
        // two further real experiments after the candidate memory; every attempt is recorded.
        let mut attempts = 0_u32;
        loop {
            attempts += 1;
            probe.wait(&mut controller, 420, |row| {
                row["decision"]["decision"] == "candidate_applied"
                    && focus_matches(row, &replacement)
            })?;
            let from = probe.mark();
            probe.wait_from(&mut controller, from, 150, |row| {
                matches!(
                    row["decision"]["decision"].as_str(),
                    Some("candidate_kept" | "candidate_reverted")
                )
            })?;
            if probe.records[from..]
                .iter()
                .any(|row| row["decision"]["decision"] == "candidate_kept")
            {
                break;
            }
            ensure!(
                attempts < 3,
                "the stable replacement under pressure was never kept in {attempts} experiments"
            );
        }
        cases.push(json!({"scenario":"stable_replacement_pressure_supported_keep","pid":replacement.child.pid(),"starttime_ticks":replacement.ticks,"experiments":attempts}));
        stop_game(&mut replacement)?;
        probe.wait(&mut controller, 40, |row| row["target"].is_null())?;
        // A higher-scoring workload may legitimately take the focus from a game mid-experiment.
        // The experiment must be rolled back and released, and the build must never be mutated.
        let mut contested = world.spawn_game(dir, "contested-game", 590)?;
        probe.own(contested.child.pid());
        probe.wait(&mut controller, 30, |row| focus_matches(row, &contested))?;
        let _pressure = world.spawn_burst(cpus.0, 8, 590)?;
        probe.wait(&mut controller, 420, |row| {
            row["decision"]["decision"] == "candidate_applied" && focus_matches(row, &contested)
        })?;
        let build = world.spawn_compile(dir, "contested-build", 90)?;
        probe.own(build.pid());
        probe.wait(&mut controller, 60, |row| {
            row["rollback_performed"] == true
                && row["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("focus_winner_changed"))
        })?;
        let rows = probe.quiet(&mut controller, 20, None)?;
        cases.push(json!({"scenario":"compile_supersedes_game_experiment","game_pid":contested.child.pid(),"build_pid":build.pid(),"observations":rows}));
        stop_game(&mut contested)?;
        drop(build);
        probe.wait(&mut controller, 40, |row| row["target"].is_null())?;
        ensure!(
            controller.child.pid() == original_pid && controller.child.try_wait()?.is_none(),
            "focus swap restarted the always-on controller"
        );
        probe.quiet(&mut controller, 6, None)?;
        ensure!(
            !world.ledger_path().exists(),
            "kept action was not released after focus left"
        );
        let report = binding.action(Action::Query)?;
        super::service::attest(&report, state)?;
        ensure!(
            report["worker_tokens"]["outstanding"] == 0,
            "worker still owns an unrecovered token after workload exit"
        );
        probe.poll();
        Ok(
            json!({"passed":true,"controller_pid":original_pid,"cases":cases,"history":probe.records,"runtime":report}),
        )
    })();
    let stopped = binding.action(Action::Stop);
    if let Err(error) = &result {
        fs::write(
            dir.join("failed-discovery.json"),
            serde_json::to_vec_pretty(
                &json!({"error":format!("{error:#}"),"history":probe.records}),
            )?,
        )?;
    }
    stopped?;
    result
}
