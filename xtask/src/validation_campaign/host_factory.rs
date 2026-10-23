//! Builds the real capture host for the `corpus` task: locates the privileged runner, discovers
//! the NIC interrupt source once and freezes the chosen IRQ in campaign state so every capture
//! (and every resumed run) uses the same one.
use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};

use stutter_validation_runner::client::SOCKET_ENV;

use super::{
    bootstrap::DEFAULT_SOCKET,
    corpus::HostFactory,
    executor::{RunContext, StateUpdate},
    model::{BlockReason, FailureClass},
    scenarios::{
        exec::CaptureHost,
        model::{InterventionKind, ScenarioSet},
    },
    system_host::{BURST_MS, BURST_PERIOD_MS, HostSettings, SystemCaptureHost},
    workloads::{
        SystemTools,
        network_irq::{self, NicSource},
    },
};

pub const FROZEN_IRQ: &str = "corpus.irq";
pub const WORKLOAD_VERSION: &str = "corpus-v1";
const PROBE_SECONDS: u64 = 5;
const PROBE_MIN_DELTA: u64 = 500;

pub struct SystemHostFactory;

fn block(class: FailureClass, detail: impl Into<String>) -> BlockReason {
    BlockReason {
        class,
        detail: detail.into(),
    }
}

fn msi_irq_names(interface: &str) -> Vec<String> {
    fs::read_dir(format!("/sys/class/net/{interface}/device/msi_irqs"))
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Which candidate IRQ rises when traffic is sent to the gateway (the independently
/// controllable source). `Err` is the honest reason there is none.
fn probe_irq(nic: &NicSource) -> Result<u32, String> {
    let read = || -> Result<BTreeMap<u32, u64>, String> {
        fs::read_to_string("/proc/interrupts")
            .map(|t| network_irq::parse_interrupts(&t))
            .map_err(|e| e.to_string())
    };
    let before = read()?;
    let target = std::net::SocketAddr::from((nic.gateway, 9));
    network_irq::send_traffic(target, 4_000, 512, PROBE_SECONDS).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(500));
    network_irq::best_irq(&before, &read()?, &nic.irqs, PROBE_MIN_DELTA)
        .map(|(irq, _)| irq)
        .ok_or_else(|| {
            format!(
                "no interrupt of {} rose by {PROBE_MIN_DELTA} under {PROBE_SECONDS}s of traffic",
                nic.interface
            )
        })
}

impl HostFactory for SystemHostFactory {
    fn host(
        &self,
        context: &RunContext<'_>,
        set: &ScenarioSet,
    ) -> Result<Box<dyn CaptureHost>, BlockReason> {
        let socket = std::env::var_os(SOCKET_ENV)
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .or_else(|| {
                PathBuf::from(DEFAULT_SOCKET)
                    .exists()
                    .then(|| PathBuf::from(DEFAULT_SOCKET))
            })
            .ok_or_else(|| {
                block(
                    FailureClass::BlockedPrivilege,
                    "the privileged validation runner is not running (bootstrap it first)",
                )
            })?;
        let needs_irq = set
            .scenarios
            .iter()
            .any(|s| s.interventions.contains(&InterventionKind::NetworkTraffic));
        let (nic, irq) = if needs_irq {
            let nic = network_irq::discover(&SystemTools, &|iface| msi_irq_names(iface))
                .map_err(|e| block(FailureClass::BlockedResource, e))?;
            let irq = match context
                .state
                .frozen
                .get(FROZEN_IRQ)
                .and_then(|v| v.parse().ok())
            {
                Some(irq) => irq,
                None => {
                    let irq =
                        probe_irq(&nic).map_err(|e| block(FailureClass::BlockedResource, e))?;
                    context.record(StateUpdate::Freeze {
                        key: FROZEN_IRQ.to_owned(),
                        value: irq.to_string(),
                    });
                    irq
                }
            };
            (Some(nic), Some(irq))
        } else {
            (None, None)
        };
        let scratch = context.dir.0.join("scratch");
        Ok(Box::new(SystemCaptureHost::new(HostSettings {
            socket,
            campaign_id: context.state.campaign_id.clone(),
            manifest_sha256: context.state.manifest_sha256.clone(),
            task: "corpus",
            task_attempt: 0,
            capture: 0,
            scratch,
            vkmark: set.vkmark.clone(),
            nic,
            irq,
            workload_version: WORKLOAD_VERSION.to_owned(),
            burst_ms: BURST_MS,
            burst_period_ms: BURST_PERIOD_MS,
        })))
    }
}

/// The real host for one frame-score degradation level: the light vkmark target plus a
/// scheduler contender of `level_ms` ms in every `PERIOD_MS`.
pub fn level_host(
    context: &RunContext<'_>,
    level_ms: u32,
) -> Result<Box<dyn CaptureHost>, BlockReason> {
    let socket = std::env::var_os(SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| {
            PathBuf::from(DEFAULT_SOCKET)
                .exists()
                .then(|| PathBuf::from(DEFAULT_SOCKET))
        })
        .ok_or_else(|| {
            block(
                FailureClass::BlockedPrivilege,
                "the privileged validation runner is not running (bootstrap it first)",
            )
        })?;
    let text = fs::read_to_string(context.repository.join(super::corpus::SCENARIO_FILE))
        .map_err(|e| block(FailureClass::BlockedResource, e.to_string()))?;
    let set = ScenarioSet::parse(&text)
        .map_err(|e| block(FailureClass::BlockedResource, e.to_string()))?;
    Ok(Box::new(SystemCaptureHost::new(HostSettings {
        socket,
        campaign_id: context.state.campaign_id.clone(),
        manifest_sha256: context.state.manifest_sha256.clone(),
        task: "frame-score",
        task_attempt: 0,
        capture: 0,
        scratch: context.dir.0.join("scratch"),
        vkmark: set.vkmark,
        nic: None,
        irq: None,
        workload_version: WORKLOAD_VERSION.to_owned(),
        burst_ms: u64::from(level_ms),
        burst_period_ms: super::frame_score::PERIOD_MS,
    })))
}
