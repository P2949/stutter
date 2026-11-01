//! A service outlives its target. Quiescence must be measured without terminating the controller.
use std::{
    fs,
    time::{Duration, Instant},
};

use anyhow::ensure;
use serde_json::Value;
use stutter_validation_runner::model::BoundedServiceAction;

use super::world::{Controller, World};

pub(super) fn wait(world: &World, controller: &mut Controller) -> anyhow::Result<Value> {
    let service = world
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing service binding"))?;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ensure!(
            controller.child.try_wait()?.is_none(),
            "always-on controller exited when its workload left"
        );
        // No journal at all is the product's own clean state (a controller that never acted).
        let clean = match fs::read(world.journal_path()) {
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .is_ok_and(|value| matches!(value["state"].as_str(), Some("clean" | "reverted"))),
        };
        if clean && !world.ledger_path().exists() {
            let report = service.action(BoundedServiceAction::Query)?;
            super::service::attest(&report, &service.campaign)?;
            ensure!(
                report["controller_process"]["pid"] == controller.child.pid(),
                "service identity changed during workload release"
            );
            ensure!(
                report["worker_tokens"]["outstanding"] == 0,
                "root worker still owns an unsettled token after workload release"
            );
            return Ok(report);
        }
        ensure!(
            Instant::now() < deadline,
            "service did not release workload actions and reach a clean journal/ledger"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
