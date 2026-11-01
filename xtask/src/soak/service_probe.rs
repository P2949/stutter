//! Reading the installed controller's decision history while a live scenario drives workloads.
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use anyhow::ensure;
use serde_json::Value;

use super::{
    history::Tail,
    world::{Controller, Game, signal, wait_exit},
};
use crate::{service_campaign::driver::StageFailure, validation_campaign::model::FailureClass};

pub(super) struct Probe {
    pub(super) tail: Tail,
    pub(super) records: Vec<Value>,
    /// Every process this acceptance started; any other high-confidence focus is foreign.
    pub(super) owned: BTreeSet<u32>,
}
impl Probe {
    pub(super) fn own(&mut self, pid: u32) {
        self.owned.insert(pid);
    }
    /// A workload this acceptance did not start that held the focus since `from`.
    pub(super) fn foreign_focus(&self, from: usize) -> Option<String> {
        self.records[from..].iter().find_map(|row| {
            let pid = row["target"]["root_pid"].as_u64()? as u32;
            let confident = row["focus"]["confidence"].as_f64()? >= 0.85;
            (confident && !self.owned.contains(&pid))
                .then(|| format!("{} pid {pid}", row["focus"]["kind"]))
        })
    }
    pub(super) fn poll(&mut self) {
        self.records.extend(self.tail.poll_records());
    }
    pub(super) fn wait(
        &mut self,
        controller: &mut Controller,
        seconds: u64,
        matches: impl Fn(&Value) -> bool,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let start = self.records.len();
        loop {
            self.poll();
            ensure!(
                controller.child.try_wait()?.is_none(),
                "packaged controller exited during discovery"
            );
            if self.records[start..].iter().any(&matches) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                if let Some(foreign) = self.foreign_focus(start) {
                    return Err(StageFailure::error(
                        FailureClass::Retryable,
                        format!(
                            "an unrelated workload held the focus ({foreign}) while waiting {seconds}s; rerun on a quiet host"
                        ),
                    ));
                }
                anyhow::bail!("live focus/decision condition did not occur within {seconds}s");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    pub(super) fn quiet(
        &mut self,
        controller: &mut Controller,
        seconds: u64,
        forbidden_pid: Option<u32>,
    ) -> anyhow::Result<Vec<Value>> {
        let start = self.records.len();
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            self.poll();
            ensure!(
                controller.child.try_wait()?.is_none(),
                "packaged controller exited during a negative focus scenario"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        self.poll();
        let rows = &self.records[start..];
        ensure!(
            !rows.is_empty(),
            "no controller observations during negative scenario"
        );
        for row in rows {
            ensure!(
                row["decision"]["decision"] != "candidate_applied",
                "an autonomous apply occurred during a negative focus scenario"
            );
            if let Some(pid) = forbidden_pid {
                ensure!(
                    row["target"]["root_pid"] != pid,
                    "an unqualified target became the controller root"
                );
            }
        }
        Ok(rows.to_vec())
    }
}

pub(super) fn stop_game(game: &mut Game) -> anyhow::Result<()> {
    signal(game.child.pid(), 15);
    ensure!(
        wait_exit(&mut game.child, Duration::from_secs(10))?.is_some(),
        "disposable workload did not exit"
    );
    Ok(())
}

pub(super) fn focus_matches(row: &Value, game: &Game) -> bool {
    row["target"]["root_pid"] == game.child.pid()
        && row["focus"]["confidence"]
            .as_f64()
            .is_some_and(|value| value >= 0.85)
        && row["focus"]["root_starttimes"][game.child.pid().to_string()] == game.ticks
}
