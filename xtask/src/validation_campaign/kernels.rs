//! Autonomous multi-kernel compatibility. The stage observes the running kernel with the
//! kernel-compat suite, and if fewer than the required number of *distinct* `uname -r` values
//! have been observed, selects the next authorized boot entry one-shot, persists that intent,
//! and reboots; after the reboot it verifies the transition, observes again, and finally
//! returns to the original entry. It stops (BlockedResource) rather than guess whenever the
//! boot environment is not exactly what was authorized.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use serde_json::Value;
use stutter_validation_runner::{
    client::{Client, SOCKET_ENV},
    model::{Request, ResponseStatus},
};

use super::{
    backend::backend_from_env,
    bootstrap::{CAMPAIGN_TASK, DEFAULT_SOCKET},
    evidence::Accept,
    executor::{AfterPersist, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::TaskSpec,
    model::{BlockReason, FailureClass},
    runners::{LiveSuiteRunner, SuiteShape},
    state::CampaignDir,
};

pub const TASK: &str = "kernel-matrix";
const PENDING_KEY: &str = "kernel.pending";
const TRIED_KEY: &str = "kernel.tried";
const OBSERVED_KEY: &str = "kernel.observed";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BootFacts {
    pub backend: String,
    pub current_entry: Option<String>,
    pub kernel_release: String,
    pub allowed: Vec<String>,
    pub original: Option<String>,
    /// Kernel version the bootloader reports per authorized entry (a hint, never authority).
    pub entry_versions: BTreeMap<String, String>,
    pub reboots_used: u32,
    pub max_reboots: u32,
}

impl BootFacts {
    /// Parses the runner's `Status` data.
    pub fn from_status(data: &Value) -> Self {
        let boot = &data["boot"];
        let strings = |value: &Value| {
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let entry_versions = boot["entries"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| {
                        Some((
                            e["id"].as_str()?.to_owned(),
                            e["version"].as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            backend: boot["backend"].as_str().unwrap_or("unsupported").to_owned(),
            current_entry: boot["current_entry"].as_str().map(str::to_owned),
            kernel_release: boot["kernel_release"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            allowed: strings(&data["allowed_boot_entries"]),
            original: data["original_boot_entry"].as_str().map(str::to_owned),
            entry_versions,
            reboots_used: data["reboots_used"].as_u64().map_or(0, |n| n as u32),
            max_reboots: data["max_reboots"].as_u64().map_or(0, |n| n as u32),
        }
    }
}

/// Everything the planner decides from.
#[derive(Clone, Debug)]
pub struct KernelView {
    pub required: usize,
    /// Distinct kernel releases already observed (from verified compat bundles).
    pub observed: BTreeSet<String>,
    pub boot: BootFacts,
    /// Entries already booted and observed in this campaign.
    pub tried: BTreeSet<String>,
    /// The entry the campaign asked to boot into, if a reboot is in flight.
    pub pending: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    /// The reboot landed where authorized; acknowledge and re-plan.
    Arrived,
    RunCompat,
    Reboot {
        into: String,
        restore: bool,
    },
    Done {
        restored: bool,
    },
    Blocked(String),
}

/// Pure decision of the next step.
pub fn plan(view: &KernelView) -> Step {
    let boot = &view.boot;
    if matches!(boot.backend.as_str(), "" | "unsupported" | "unknown") {
        return Step::Blocked(
            "no supported bootloader: autonomous kernel selection is unavailable".to_owned(),
        );
    }
    if let Some(pending) = &view.pending {
        return if boot.current_entry.as_ref() == Some(pending) {
            Step::Arrived
        } else {
            Step::Blocked(format!(
                "booted into {:?} but the authorized transition was to {pending}; stopping autonomous rebooting",
                boot.current_entry
            ))
        };
    }
    if !view.observed.contains(&boot.kernel_release) {
        return Step::RunCompat;
    }
    let budget_left = boot.reboots_used < boot.max_reboots;
    if view.observed.len() >= view.required {
        return match (&boot.original, &boot.current_entry) {
            (Some(original), Some(current)) if original == current => Step::Done { restored: true },
            (Some(original), _) if budget_left => Step::Reboot {
                into: original.clone(),
                restore: true,
            },
            (Some(_), _) => Step::Blocked(
                "the reboot budget is spent before the original boot entry was restored".to_owned(),
            ),
            // The default entry was never changed, so the next ordinary boot is the user's own.
            (None, _) => Step::Done { restored: false },
        };
    }
    let next = boot.allowed.iter().find(|entry| {
        !view.tried.contains(*entry)
            && boot.current_entry.as_ref() != Some(*entry)
            // An entry whose kernel version equals the running one cannot add a new release.
            && boot.entry_versions.get(*entry) != Some(&boot.kernel_release)
    });
    match next {
        None => Step::Blocked(format!(
            "all candidate boot entries were tried with {} of {} distinct kernels observed",
            view.observed.len(),
            view.required
        )),
        Some(_) if !budget_left => Step::Blocked(format!(
            "the reboot budget ({}) is exhausted with {} of {} distinct kernels observed",
            boot.max_reboots,
            view.observed.len(),
            view.required
        )),
        Some(entry) => Step::Reboot {
            into: entry.clone(),
            restore: false,
        },
    }
}

/// Kernel releases (and their observed compatibility) from the campaign's published,
/// verified kernel-compat entries. Receipts, not cached state, are the truth.
pub fn observed_kernels(dir: &CampaignDir) -> BTreeMap<String, String> {
    let mut kernels = BTreeMap::new();
    let root = dir.indexes().join(TASK);
    let Ok(read) = fs::read_dir(&root) else {
        return kernels;
    };
    let mut files = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    files.sort();
    for file in files {
        let Ok(bytes) = fs::read(&file) else { continue };
        let Ok(entry) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let (Some(kernel), Some(state)) = (
            entry["kernel_release"].as_str(),
            entry["compatibility"].as_str(),
        ) {
            kernels.insert(kernel.to_owned(), state.to_owned());
        }
    }
    kernels
}

#[derive(Debug)]
pub enum HostError {
    Privilege(String),
    Failed(String),
}

/// One compat observation just published.
#[derive(Clone, Debug)]
pub struct CompatObservation {
    pub kernel_release: String,
}

/// The machine and runner the stage acts on. A trait so the whole state machine, including
/// reboots, is simulated in CI with a fake machine.
pub trait KernelHost {
    fn boot_facts(&self, ctx: &RunContext<'_>) -> Result<BootFacts, HostError>;
    fn run_compat(
        &self,
        ctx: &RunContext<'_>,
        spec: &TaskSpec,
        attempt: u32,
    ) -> Result<CompatObservation, RunnerOutcome>;
    fn set_next_boot(&self, ctx: &RunContext<'_>, entry: &str) -> Result<(), HostError>;
    fn reboot_action(&self, ctx: &RunContext<'_>) -> AfterPersist;
    fn acknowledge_resume(&self, ctx: &RunContext<'_>) -> Result<(), HostError>;
}

fn blocked(class: FailureClass, detail: impl Into<String>) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason {
        class,
        detail: detail.into(),
    })
}

fn host_blocked(error: HostError) -> RunnerOutcome {
    match error {
        HostError::Privilege(detail) => blocked(FailureClass::BlockedPrivilege, detail),
        HostError::Failed(detail) => blocked(FailureClass::BlockedResource, detail),
    }
}

fn split(list: Option<&String>) -> BTreeSet<String> {
    list.map(|value| {
        value
            .split(',')
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(",")
}

pub struct KernelMatrixRunner {
    pub host: Box<dyn KernelHost>,
}

impl TaskRunner for KernelMatrixRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let required = context.manifest.kernels.required_distinct;
        let mut tried = split(context.state.frozen.get(TRIED_KEY));
        let mut pending = Some(
            context
                .state
                .frozen
                .get(PENDING_KEY)
                .cloned()
                .unwrap_or_default(),
        )
        .filter(|value| !value.is_empty());
        let mut compat_ran = false;
        // Each iteration either returns or advances one step; the bound only guards a bug.
        for _ in 0..6 {
            let boot = match self.host.boot_facts(context) {
                Ok(boot) => boot,
                Err(error) => return host_blocked(error),
            };
            let observed = observed_kernels(context.dir);
            let view = KernelView {
                required,
                observed: observed.keys().cloned().collect(),
                boot: boot.clone(),
                tried: tried.clone(),
                pending: pending.clone(),
            };
            match plan(&view) {
                Step::Arrived => {
                    if let Err(error) = self.host.acknowledge_resume(context) {
                        return host_blocked(error);
                    }
                    context.record(StateUpdate::Freeze {
                        key: PENDING_KEY.to_owned(),
                        value: String::new(),
                    });
                    pending = None;
                }
                Step::RunCompat => {
                    if compat_ran {
                        return blocked(
                            FailureClass::BlockedResource,
                            "the kernel was observed twice without progress",
                        );
                    }
                    match self.host.run_compat(context, spec, attempt) {
                        Ok(observation) => {
                            compat_ran = true;
                            if let Some(entry) = &boot.current_entry {
                                tried.insert(entry.clone());
                            }
                            context.record(StateUpdate::Freeze {
                                key: TRIED_KEY.to_owned(),
                                value: join(&tried),
                            });
                            let mut kernels = observed.keys().cloned().collect::<BTreeSet<_>>();
                            kernels.insert(observation.kernel_release);
                            context.record(StateUpdate::Freeze {
                                key: OBSERVED_KEY.to_owned(),
                                value: join(&kernels),
                            });
                        }
                        Err(outcome) => return outcome,
                    }
                }
                Step::Reboot { into, restore } => {
                    if let Err(error) = self.host.set_next_boot(context, &into) {
                        return host_blocked(error);
                    }
                    context.record(StateUpdate::Freeze {
                        key: PENDING_KEY.to_owned(),
                        value: into,
                    });
                    if !restore {
                        context.record(StateUpdate::Freeze {
                            key: TRIED_KEY.to_owned(),
                            value: join(&tried),
                        });
                    }
                    return RunnerOutcome::waiting_for_reboot(self.host.reboot_action(context));
                }
                Step::Done { restored } => {
                    let limitations = if restored {
                        Vec::new()
                    } else {
                        vec!["the original boot entry could not be verified; the default entry was never changed".to_owned()]
                    };
                    context.record(StateUpdate::Freeze {
                        key: PENDING_KEY.to_owned(),
                        value: String::new(),
                    });
                    return RunnerOutcome::Passed {
                        artifacts: None,
                        receipt: None,
                        limitations,
                        fingerprint: Default::default(),
                        updates: Vec::new(),
                    };
                }
                Step::Blocked(reason) => return blocked(FailureClass::BlockedResource, reason),
            }
        }
        blocked(
            FailureClass::BlockedResource,
            "the kernel stage did not converge",
        )
    }
}

/// The real host: the validation runner for boot control and the kernel-compat suite.
pub struct RunnerHost;

fn socket_path() -> std::path::PathBuf {
    std::env::var_os(SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::path::PathBuf::from(DEFAULT_SOCKET),
            std::path::PathBuf::from,
        )
}

fn send(
    socket: &std::path::Path,
    campaign: &str,
    manifest: &str,
    task: &str,
    request: Request,
) -> Result<stutter_validation_runner::model::Response, HostError> {
    let client = Client {
        socket,
        campaign_id: campaign,
        manifest_sha256: manifest,
    };
    let response = client.send(task, request).map_err(|error| {
        HostError::Privilege(format!(
            "the validation runner at {} is unreachable: {error}",
            socket.display()
        ))
    })?;
    if response.status == ResponseStatus::Ok {
        return Ok(response);
    }
    let error = response
        .error
        .map_or_else(String::new, |e| format!("{:?}: {}", e.code, e.message));
    Err(HostError::Failed(format!("runner refused: {error}")))
}

impl KernelHost for RunnerHost {
    fn boot_facts(&self, ctx: &RunContext<'_>) -> Result<BootFacts, HostError> {
        let response = send(
            &socket_path(),
            &ctx.state.campaign_id,
            &ctx.state.manifest_sha256,
            CAMPAIGN_TASK,
            Request::Status,
        )?;
        Ok(BootFacts::from_status(&response.data.unwrap_or_default()))
    }

    fn run_compat(
        &self,
        ctx: &RunContext<'_>,
        spec: &TaskSpec,
        attempt: u32,
    ) -> Result<CompatObservation, RunnerOutcome> {
        let runner = LiveSuiteRunner {
            suite: "kernel-compat",
            shape: SuiteShape::Seconds,
            backend: backend_from_env(),
        };
        let published =
            runner.run_and_publish(ctx, spec, attempt, Accept::PassOrObservedIncompatible)?;
        Ok(CompatObservation {
            kernel_release: published.index_entry["kernel_release"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        })
    }

    fn set_next_boot(&self, ctx: &RunContext<'_>, entry: &str) -> Result<(), HostError> {
        send(
            &socket_path(),
            &ctx.state.campaign_id,
            &ctx.state.manifest_sha256,
            TASK,
            Request::SetNextBoot {
                entry_id: entry.to_owned(),
            },
        )
        .map(drop)
    }

    fn reboot_action(&self, ctx: &RunContext<'_>) -> AfterPersist {
        let (campaign, manifest) = (
            ctx.state.campaign_id.clone(),
            ctx.state.manifest_sha256.clone(),
        );
        Box::new(move || {
            send(&socket_path(), &campaign, &manifest, TASK, Request::Reboot)
                .map(drop)
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn acknowledge_resume(&self, ctx: &RunContext<'_>) -> Result<(), HostError> {
        send(
            &socket_path(),
            &ctx.state.campaign_id,
            &ctx.state.manifest_sha256,
            TASK,
            Request::ResumeCampaign,
        )
        .map(drop)
    }
}

/// Best-effort cancellation of the runner's side (drops any pending one-shot boot selection and
/// future work). `None` when no runner is reachable, which is not an error for a cancel.
pub fn cancel_runner(state: &super::model::CampaignState) -> Option<String> {
    let socket = socket_path();
    if !socket.exists() {
        return None;
    }
    Some(
        match send(
            &socket,
            &state.campaign_id,
            &state.manifest_sha256,
            CAMPAIGN_TASK,
            Request::CancelCampaign,
        ) {
            Ok(_) => "the validation runner was cancelled and any pending boot selection dropped"
                .to_owned(),
            Err(error) => format!("the validation runner could not be cancelled: {error:?}"),
        },
    )
}
