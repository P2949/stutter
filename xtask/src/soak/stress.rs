//! `xtask soak controller-stop-stress`: the real-controller regression gate for the SIGINT race
//! fixed in #178. The unit test proves the signal primitive; this proves the shutdown path end to
//! end: the production controller and privileged worker, a cgroup-placement experiment applied,
//! SIGINT mid-experiment, a normal exit, the placement physically restored, a clean journal and
//! an empty kept-action ledger. It reuses the soak's own round machinery and cgroup delegation.
//! Every iteration starts from a fresh isolated `HOME`, so the controller's five-minute
//! candidate memory never serialises the iterations.
use std::{fs, path::Path, sync::atomic::Ordering};

use anyhow::{bail, ensure};

use super::{
    plan::{self, Kind, Operations, Round, Scenario},
    records::{Profile, RoundRecord, unix_ms},
    round,
    world::World,
};

/// What one iteration must show; empty when it did.
pub(super) fn iteration_problems(record: &RoundRecord) -> Vec<String> {
    let mut problems = Vec::new();
    if record.outcome != "ok" {
        problems.push(format!(
            "the round ended `{}`: {}",
            record.outcome,
            record.error.clone().unwrap_or_default()
        ));
    }
    problems.extend(record.violations.iter().cloned());
    let has = |kind: &str| record.events.iter().any(|event| event.kind == kind);
    if !has("candidate_applied") {
        problems.push("no candidate was applied before the interrupt".to_owned());
    }
    if !has("controller_sigint") {
        problems.push("the SIGINT was never sent".to_owned());
    }
    match record.sessions.first() {
        Some(session) if session.ended_by == "sigint" && session.exit_code == Some(0) => {}
        Some(session) => problems.push(format!(
            "the interrupted controller ended `{}` with exit {:?}, not a normal exit",
            session.ended_by, session.exit_code
        )),
        None => problems.push("no controller session was recorded".to_owned()),
    }
    problems
}

pub(super) fn controller_stop_stress(
    root: &Path,
    iterations: u32,
    stutter_bin: &Path,
) -> anyhow::Result<()> {
    ensure!(iterations > 0, "the stress needs at least one iteration");
    let stutter = root.join(stutter_bin);
    ensure!(stutter.is_file(), "{} is not built", stutter.display());
    if !super::delegated() {
        return super::reexec_delegated(
            root,
            &[
                "soak".into(),
                "controller-stop-stress".into(),
                "--iterations".into(),
                iterations.to_string().into(),
                "--stutter-bin".into(),
                stutter.into_os_string(),
            ],
        );
    }
    super::world::require_tools()?;
    super::install_signal_handlers();
    let cpus = super::pick_cpus()?;
    let base = root
        .join("target/controller-stop-stress")
        .join(unix_ms().to_string());
    let spec = Round {
        kind: Kind::ControllerStop,
        scenario: Scenario::Beneficial,
        operations: Operations::Cgroup,
    };
    let mut failed = Vec::new();
    for index in 0..iterations {
        if round::STOP.load(Ordering::SeqCst) {
            bail!("interrupted before iteration {index}");
        }
        let dir = base.join(format!("iteration-{index:02}"));
        let mut world = World::new(root, &dir, &stutter, cpus)?;
        super::clear_leftovers();
        let (record, _) =
            round::run_round(&mut world, &dir, index, spec, plan::timing(Profile::Smoke));
        world.kill_worker();
        let problems = iteration_problems(&record);
        println!(
            "iteration {index}: {}",
            if problems.is_empty() { "ok" } else { "FAILED" }
        );
        for problem in &problems {
            println!("  {problem}");
        }
        if !problems.is_empty() {
            failed.push(index);
        }
    }
    super::cleanup_cgroup();
    ensure!(
        failed.is_empty(),
        "controller-stop stress failed in iteration(s) {failed:?}; state kept in {}",
        base.display()
    );
    let _ = fs::remove_dir_all(&base);
    println!("controller-stop-stress: PASS ({iterations} iterations)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soak::records::{EventRecord, SessionRecord};

    fn good() -> RoundRecord {
        RoundRecord {
            outcome: "ok".to_owned(),
            sessions: vec![SessionRecord {
                ended_by: "sigint".to_owned(),
                exit_code: Some(0),
                ..SessionRecord::default()
            }],
            events: ["candidate_applied", "controller_sigint"]
                .iter()
                .map(|kind| EventRecord {
                    kind: (*kind).to_owned(),
                    ..EventRecord::default()
                })
                .collect(),
            ..RoundRecord::default()
        }
    }

    #[test]
    fn a_clean_interrupt_after_an_apply_passes() {
        assert!(iteration_problems(&good()).is_empty());
    }

    #[test]
    fn every_way_the_shutdown_can_go_wrong_fails_the_iteration() {
        let mut stuck = good();
        stuck.sessions[0].exit_code = None; // killed after the 45 s wait: the #178 symptom
        assert!(!iteration_problems(&stuck).is_empty());
        let mut crashed = good();
        crashed.sessions[0].exit_code = Some(1);
        assert!(!iteration_problems(&crashed).is_empty());
        let mut unapplied = good();
        unapplied
            .events
            .retain(|event| event.kind != "candidate_applied");
        assert!(!iteration_problems(&unapplied).is_empty());
        let mut unsent = good();
        unsent
            .events
            .retain(|event| event.kind != "controller_sigint");
        assert!(!iteration_problems(&unsent).is_empty());
        let mut dirty = good();
        dirty
            .violations
            .push("the controller journal is `keeping` at rest".to_owned());
        assert!(!iteration_problems(&dirty).is_empty());
        let mut failed = good();
        failed.outcome = "failed".to_owned();
        assert!(!iteration_problems(&failed).is_empty());
        let mut none = good();
        none.sessions.clear();
        assert!(!iteration_problems(&none).is_empty());
    }
}
