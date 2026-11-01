//! Adapter from the existing soak world to the actual packaged service and typed root runner.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stutter_validation_runner::model::{
    BoundedServiceAction as Action, Request, ServiceComponent, ServiceOperations, ServiceSignal,
};

use super::{plan::Operations, world::start_ticks};
use crate::service_campaign::{
    model::{Stage, State},
    runner,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Binding {
    pub campaign: State,
    pub stage: Stage,
    pub user: String,
    #[serde(skip)]
    pub(super) presence: Option<std::sync::Arc<super::service_presence::Presence>>,
}

impl Binding {
    pub(crate) fn new(campaign: &State, stage: Stage) -> anyhow::Result<Self> {
        let user = stutter_validation_runner::service::account_name(
            &fs::read_to_string("/etc/passwd")?,
            campaign.uid,
            campaign.gid,
        )
        .map_err(|(_, message)| anyhow::anyhow!(message))?;
        Ok(Self {
            campaign: campaign.clone(),
            stage,
            user,
            presence: None,
        })
    }
    pub(super) fn action(&self, action: Action) -> anyhow::Result<Value> {
        runner::request(
            &self.campaign,
            self.stage,
            Request::BoundedService { action },
        )
    }
    pub(super) fn home(&self) -> PathBuf {
        Path::new("/var/lib/stutter").join(format!("autotune-{}", self.user))
    }
    pub(super) fn prepare(&self, cpus: (u32, u32), operations: Operations) -> anyhow::Result<()> {
        self.action(Action::Stop)?;
        self.action(Action::PrepareFixture {
            source_cpu: cpus.0,
            destination_cpu: cpus.1,
            operations: match operations {
                Operations::Affinity => ServiceOperations::Affinity,
                Operations::Cgroup => ServiceOperations::Cgroup,
                Operations::Both => ServiceOperations::Both,
            },
        })?;
        Ok(())
    }
    pub(super) fn attach(&self, pid: u32, ticks: u64) -> anyhow::Result<()> {
        self.action(Action::AttachFixture {
            pid,
            starttime_ticks: ticks,
        })?;
        Ok(())
    }
    pub(super) fn start(&self) -> anyhow::Result<Process> {
        self.action(Action::Start)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let report = self.action(Action::Query)?;
            if attest(&report, &self.campaign).is_ok() {
                if let Some(presence) = &self.presence {
                    presence.observe(&report)?;
                }
                return Ok(Process {
                    binding: self.clone(),
                    pid: report["controller_process"]["pid"]
                        .as_u64()
                        .context("missing controller PID")? as u32,
                    ticks: report["controller_process"]["starttime_ticks"]
                        .as_u64()
                        .context("missing start time")?,
                    boot_id: report["boot_id"]
                        .as_str()
                        .context("missing boot identity")?
                        .into(),
                    worker_identity: (
                        report["worker_process"]["pid"]
                            .as_u64()
                            .context("missing worker PID")? as u32,
                        report["worker_process"]["starttime_ticks"]
                            .as_u64()
                            .context("missing worker start ticks")?,
                    ),
                });
            }
            ensure!(
                Instant::now() < deadline,
                "packaged service did not reach a stable runtime identity: {report}"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    pub(super) fn burst(
        &self,
        cpu: u32,
        burst_ms: u64,
        seconds: u64,
    ) -> anyhow::Result<Contention> {
        let result = runner::request(
            &self.campaign,
            self.stage,
            Request::ExecuteSchedulerBurst {
                cpu,
                burst_ms,
                period_ms: 20,
                seconds,
            },
        )?;
        let pid = result["pid"].as_u64().context("missing burst PID")? as u32;
        Ok(Contention {
            pid,
            ticks: start_ticks(pid)?,
        })
    }
}

/// Rich root attestation is performed at service boundaries, not on every sampling poll.
pub(crate) fn attest(report: &Value, state: &State) -> anyhow::Result<()> {
    ensure!(
        report["ownership_verified"] == true,
        "service unit/configuration ownership is unverified"
    );
    ensure!(
        report["controller_process"]["consistent"] == true
            && report["worker_process"]["consistent"] == true,
        "service process identity was unstable"
    );
    for actor in ["controller_process", "worker_process"] {
        ensure!(
            report[actor]["binary_sha256"] == state.stutter_sha256,
            "service actor binary differs from pinned source"
        );
    }
    let controller = report["controller_process"]["status"]
        .as_object()
        .context("missing controller status")?;
    let ids: Vec<_> = controller["Uid"]
        .as_str()
        .context("missing controller UIDs")?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<_, _>>()?;
    ensure!(
        ids.len() == 4 && ids.iter().all(|uid| *uid == state.uid),
        "controller did not run as the monitored desktop user"
    );
    ensure!(
        controller["NoNewPrivs"] == "1",
        "controller lacks NoNewPrivileges"
    );
    // CAP_DAC_READ_SEARCH(2), SYS_RESOURCE(24), PERFMON(38), BPF(39), and no others.
    let expected = (1_u64 << 2) | (1_u64 << 24) | (1_u64 << 38) | (1_u64 << 39);
    for field in ["CapEff", "CapBnd", "CapAmb"] {
        let actual = u64::from_str_radix(
            controller[field]
                .as_str()
                .context("missing capability readback")?,
            16,
        )?;
        ensure!(
            actual == expected,
            "controller capability set {field} differs: {actual:x}"
        );
    }
    ensure!(
        report["worker_process"]["status"]["Uid"]
            .as_str()
            .is_some_and(|ids| ids.split_whitespace().all(|uid| uid == "0")),
        "worker was not root"
    );
    // CHOWN(0), DAC_OVERRIDE(1), SYS_PTRACE(19), SYS_NICE(23): the worker's whole bounding set.
    let worker_bound = (1_u64 << 0) | (1_u64 << 1) | (1_u64 << 19) | (1_u64 << 23);
    let bounded = report["worker_process"]["status"]["CapBnd"]
        .as_str()
        .and_then(|bound| u64::from_str_radix(bound, 16).ok());
    ensure!(
        bounded == Some(worker_bound),
        "worker capability bounding set differs from the hardened set: {bounded:x?}"
    );
    // What the worker can actually exercise, not just what the unit declares.
    let effective = report["worker_process"]["status"]["CapEff"]
        .as_str()
        .and_then(|value| u64::from_str_radix(value, 16).ok());
    ensure!(
        effective == Some(worker_bound),
        "worker CapEff differs from the hardened set: {effective:x?}"
    );
    ensure!(
        report["worker_process"]["status"]["CapAmb"]
            .as_str()
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            == Some(0)
            && report["worker_process"]["status"]["NoNewPrivs"] == "1",
        "worker holds ambient capabilities or can gain privileges"
    );
    ensure!(
        report["socket"]["uid"] == state.uid
            && report["socket"]["mode"] == 0o600
            && report["socket"]["is_socket"] == true,
        "service socket ownership differs"
    );
    ensure!(
        report["worker_tokens"]["readable"] == true,
        "root worker ledger unreadable"
    );
    ensure!(
        report["worker_tokens"]["recovery_audit"]["readable"] == true,
        "root worker recovery audit unreadable"
    );
    Ok(())
}

pub(super) struct Process {
    pub(super) binding: Binding,
    pub(super) pid: u32,
    pub(super) ticks: u64,
    boot_id: String,
    pub(super) worker_identity: (u32, u64),
}
impl Process {
    pub(super) fn alive(&self) -> bool {
        stutter_core::boot::current_boot_id().ok().as_ref() == Some(&self.boot_id)
            && start_ticks(self.pid).ok() == Some(self.ticks)
            && fs::read_to_string(format!("/proc/{}/stat", self.pid))
                .ok()
                .is_some_and(|text| {
                    text.rsplit_once(") ")
                        .is_some_and(|(_, fields)| !fields.starts_with('Z'))
                })
    }
    pub(super) fn signal(&self, signal: i32) -> anyhow::Result<()> {
        ensure!(
            self.alive(),
            "refusing signal after service identity changed"
        );
        let signal = match signal {
            2 => ServiceSignal::Int,
            9 => ServiceSignal::Kill,
            15 => ServiceSignal::Term,
            _ => anyhow::bail!("unregistered service signal"),
        };
        self.binding.action(Action::Signal {
            component: ServiceComponent::Controller,
            signal,
        })?;
        Ok(())
    }
}

/// The runner reaps its child. The same desktop UID can terminate only this identity on drop.
pub(super) struct Contention {
    pid: u32,
    ticks: u64,
}
impl Drop for Contention {
    fn drop(&mut self) {
        if start_ticks(self.pid).ok() == Some(self.ticks) {
            super::world::signal(self.pid, 9);
        }
    }
}
