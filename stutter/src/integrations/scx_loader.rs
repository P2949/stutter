//! A gated, offline strategy adapter for `scx_loader` (package W11b).
//!
//! `org.scx.Loader` is the documented way to start, switch and stop a sched_ext scheduler. This
//! adapter reads its state, checks the installed method signatures by introspection, and plans a
//! switch only to an allowlisted scheduler that the manager supports and only when the exact prior
//! state can be restored. Executing a plan needs an [`OfflineAuthorization`], which currently has
//! only a fixture constructor: there is no implemented trusted campaign authority. No runtime path, provider or remote caller can switch
//! the machine's scheduler. Nothing here parses `scxctl` output or changes dispatch maps.
use std::collections::BTreeMap;

use super::dbus::{DbusClient, DbusError, Value, Writer};

pub const SERVICE: &str = "org.scx.Loader";
pub const PATH: &str = "/org/scx/Loader";
pub const INTERFACE: &str = "org.scx.Loader";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScxLoaderState {
    /// `None` when no SCX scheduler is running (the kernel's default scheduler is in charge).
    pub current: Option<String>,
    pub mode: Option<u32>,
    pub supported: Vec<String>,
}

fn scheduler_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    (!name.is_empty() && !matches!(name.to_ascii_lowercase().as_str(), "unknown" | "none"))
        .then(|| name.to_owned())
}

pub fn read_state(client: &mut DbusClient) -> Result<ScxLoaderState, DbusError> {
    let current = match client.get_property(SERVICE, PATH, INTERFACE, "CurrentScheduler")? {
        Value::Str(text) => scheduler_name(&text),
        _ => return Err(DbusError::UnexpectedReply),
    };
    let mode = match client.get_property(SERVICE, PATH, INTERFACE, "SchedulerMode") {
        Ok(Value::U32(mode)) => Some(mode),
        Ok(_) | Err(_) => None,
    };
    let supported = match client.get_property(SERVICE, PATH, INTERFACE, "SupportedSchedulers")? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| match item {
                Value::Str(text) => scheduler_name(&text),
                _ => None,
            })
            .collect(),
        _ => return Err(DbusError::UnexpectedReply),
    };
    Ok(ScxLoaderState {
        current,
        mode,
        supported,
    })
}

/// Input signatures of the methods of `interface` in an introspection document.
pub fn method_signatures(xml: &str, interface: &str) -> BTreeMap<String, String> {
    let mut methods = BTreeMap::new();
    let mut in_interface = false;
    let mut current: Option<String> = None;
    for line in xml.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("<interface name=\"") {
            in_interface = rest.split('"').next() == Some(interface);
        } else if line.starts_with("</interface>") {
            in_interface = false;
        } else if in_interface && let Some(rest) = line.strip_prefix("<method name=\"") {
            let name = rest.split('"').next().unwrap_or_default().to_owned();
            methods.insert(name.clone(), String::new());
            current = (!line.ends_with("/>")).then_some(name);
        } else if line.starts_with("</method>") {
            current = None;
        } else if in_interface
            && let Some(name) = &current
            && line.starts_with("<arg ")
            && !line.contains("direction=\"out\"")
            && let Some(kind) = line
                .split("type=\"")
                .nth(1)
                .and_then(|t| t.split('"').next())
        {
            methods
                .entry(name.clone())
                .and_modify(|signature| signature.push_str(kind));
        }
    }
    methods
}

pub fn introspect(client: &mut DbusClient) -> Result<BTreeMap<String, String>, DbusError> {
    let values = client.call(
        SERVICE,
        PATH,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        "",
        &[],
    )?;
    match values.first() {
        Some(Value::Str(xml)) => Ok(method_signatures(xml, INTERFACE)),
        _ => Err(DbusError::UnexpectedReply),
    }
}

/// Arguments a scheduler may be started with: each entry is one exact argument list that was
/// reviewed for that scheduler. Anything else is refused, so no free-form flag reaches the loader.
pub type ArgAllowlist = Vec<(String, Vec<Vec<String>>)>;

/// Plans a switch to `to` with `args`, which must equal one reviewed entry for `to`. Uses
/// `SwitchSchedulerWithArgs` / `StartSchedulerWithArgs` (`sas`); the prior state is restored exactly
/// as for a plain switch. Empty `args` is a plain switch and goes through [`plan_switch`].
pub fn plan_switch_with_args(
    state: &ScxLoaderState,
    methods: &BTreeMap<String, String>,
    to: &str,
    mode: u32,
    args: &[String],
    allowlist: &[String],
    arg_allowlist: &ArgAllowlist,
) -> Result<SwitchPlan, SwitchDenial> {
    if args.is_empty() {
        return plan_switch(state, methods, to, mode, allowlist);
    }
    if !allowlist.iter().any(|allowed| allowed == to) {
        return Err(SwitchDenial::NotAllowlisted);
    }
    if !state.supported.iter().any(|supported| supported == to) {
        return Err(SwitchDenial::UnsupportedScheduler);
    }
    let reviewed = arg_allowlist
        .iter()
        .find(|(name, _)| name == to)
        .is_some_and(|(_, lists)| lists.iter().any(|list| list == args));
    if !reviewed {
        return Err(SwitchDenial::ArgumentsNotAllowlisted);
    }
    let need = |method: &'static str, signature: &str| -> Result<(), SwitchDenial> {
        match methods.get(method) {
            None => Err(SwitchDenial::MethodMissing(method)),
            Some(found) if found != signature => Err(SwitchDenial::SignatureMismatch {
                method,
                found: found.clone(),
            }),
            Some(_) => Ok(()),
        }
    };
    let (method, restore) = match (&state.current, state.mode) {
        (None, _) => {
            need("StartSchedulerWithArgs", "sas")?;
            need("StopScheduler", "").map_err(|_| SwitchDenial::UnsupportedRestore)?;
            ("StartSchedulerWithArgs", Restore::Stop)
        }
        (Some(prior), Some(prior_mode)) => {
            need("SwitchSchedulerWithArgs", "sas")?;
            need("SwitchScheduler", "su").map_err(|_| SwitchDenial::UnsupportedRestore)?;
            (
                "SwitchSchedulerWithArgs",
                Restore::SwitchBack {
                    name: prior.clone(),
                    mode: prior_mode,
                },
            )
        }
        (Some(_), None) => return Err(SwitchDenial::UnsupportedRestore),
    };
    Ok(SwitchPlan {
        to: to.to_owned(),
        mode,
        method,
        restore,
        args: args.to_vec(),
    })
}

/// Reserved proof for a trusted offline campaign. Production construction is deliberately
/// unavailable until the campaign policy, worker, journal and exact ownership path exist.
/// A campaign name alone is not authorization.
///
/// ```compile_fail
/// use stutter::integrations::scx_loader::OfflineAuthorization;
/// let forged = OfflineAuthorization::from_campaign("caller-selected-name");
/// ```
#[derive(Debug)]
pub struct OfflineAuthorization {
    campaign: String,
}

/// What a rollback token carries so the prior scheduler state can be put back exactly.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScxRestoreRecord {
    pub applied: String,
    pub restore: Restore,
}

impl SwitchPlan {
    pub fn restore_record(&self) -> ScxRestoreRecord {
        ScxRestoreRecord {
            applied: self.to.clone(),
            restore: self.restore.clone(),
        }
    }
}

impl OfflineAuthorization {
    /// Fake-bus fixtures only; never a constructor in the product library.
    #[cfg(test)]
    pub(super) fn from_campaign(campaign: &str) -> Self {
        Self {
            campaign: campaign.to_owned(),
        }
    }

    pub fn campaign(&self) -> &str {
        &self.campaign
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Restore {
    /// No SCX scheduler was running: stopping the one started returns to the kernel default.
    Stop,
    /// Switching back to the prior scheduler and mode.
    SwitchBack { name: String, mode: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchPlan {
    pub to: String,
    pub mode: u32,
    /// `StartScheduler` when nothing runs, `SwitchScheduler` when something does.
    pub method: &'static str,
    pub restore: Restore,
    /// Extra arguments for the scheduler, from the per-scheduler allowlist; empty for a plain switch.
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchDenial {
    NotAllowlisted,
    /// The argument list is not one of the reviewed lists for this scheduler.
    ArgumentsNotAllowlisted,
    UnsupportedScheduler,
    AlreadyRunning,
    MethodMissing(&'static str),
    SignatureMismatch {
        method: &'static str,
        found: String,
    },
    /// The prior state cannot be put back exactly (typed `UnsupportedRestore`).
    UnsupportedRestore,
}

/// Plans a switch to `to` in `mode`, or says exactly why not.
pub fn plan_switch(
    state: &ScxLoaderState,
    methods: &BTreeMap<String, String>,
    to: &str,
    mode: u32,
    allowlist: &[String],
) -> Result<SwitchPlan, SwitchDenial> {
    if !allowlist.iter().any(|allowed| allowed == to) {
        return Err(SwitchDenial::NotAllowlisted);
    }
    if !state.supported.iter().any(|supported| supported == to) {
        return Err(SwitchDenial::UnsupportedScheduler);
    }
    if state.current.as_deref() == Some(to) && state.mode == Some(mode) {
        return Err(SwitchDenial::AlreadyRunning);
    }
    let need = |method: &'static str, signature: &str| -> Result<(), SwitchDenial> {
        match methods.get(method) {
            None => Err(SwitchDenial::MethodMissing(method)),
            Some(found) if found != signature => Err(SwitchDenial::SignatureMismatch {
                method,
                found: found.clone(),
            }),
            Some(_) => Ok(()),
        }
    };
    let (method, restore) = match (&state.current, state.mode) {
        (None, _) => {
            need("StartScheduler", "su")?;
            need("StopScheduler", "").map_err(|_| SwitchDenial::UnsupportedRestore)?;
            ("StartScheduler", Restore::Stop)
        }
        (Some(prior), Some(prior_mode)) => {
            need("SwitchScheduler", "su")?;
            (
                "SwitchScheduler",
                Restore::SwitchBack {
                    name: prior.clone(),
                    mode: prior_mode,
                },
            )
        }
        // A running scheduler whose mode is unknown cannot be restored exactly.
        (Some(_), None) => return Err(SwitchDenial::UnsupportedRestore),
    };
    Ok(SwitchPlan {
        to: to.to_owned(),
        mode,
        method,
        restore,
        args: Vec::new(),
    })
}

#[derive(Debug)]
pub enum ExecuteError {
    Dbus(DbusError),
    /// The call returned but the loader (or the kernel) does not report the scheduler.
    NotInEffect {
        wanted: String,
        current: Option<String>,
    },
}

impl From<DbusError> for ExecuteError {
    fn from(error: DbusError) -> Self {
        Self::Dbus(error)
    }
}

fn call_switch(
    client: &mut DbusClient,
    method: &str,
    name: &str,
    mode: u32,
) -> Result<(), DbusError> {
    let mut body = Writer::default();
    body.string(name);
    body.u32(mode);
    client
        .call(SERVICE, PATH, INTERFACE, method, "su", &body.buf)
        .map(drop)
}

fn call_switch_with_args(
    client: &mut DbusClient,
    method: &str,
    name: &str,
    args: &[String],
) -> Result<(), DbusError> {
    let mut body = Writer::default();
    body.string(name);
    body.string_array(args);
    client
        .call(SERVICE, PATH, INTERFACE, method, "sas", &body.buf)
        .map(drop)
}

/// Performs the planned switch and verifies the loader reports it and the kernel agrees
/// (`kernel_ops` reads `/sys/kernel/sched_ext/root/ops`).
pub fn execute(
    client: &mut DbusClient,
    plan: &SwitchPlan,
    _authorization: &OfflineAuthorization,
    kernel_ops: impl Fn() -> Option<String>,
) -> Result<(), ExecuteError> {
    if plan.args.is_empty() {
        call_switch(client, plan.method, &plan.to, plan.mode)?;
    } else {
        call_switch_with_args(client, plan.method, &plan.to, &plan.args)?;
    }
    let state = read_state(client)?;
    let kernel_agrees = kernel_ops().is_some_and(|ops| {
        ops == plan.to || ops == plan.to.strip_prefix("scx_").unwrap_or(&plan.to)
    });
    if state.current.as_deref() == Some(plan.to.as_str())
        && state.mode == Some(plan.mode)
        && kernel_agrees
    {
        Ok(())
    } else {
        Err(ExecuteError::NotInEffect {
            wanted: plan.to.clone(),
            current: state.current,
        })
    }
}

/// Puts the prior state back and verifies it.
pub fn restore(
    client: &mut DbusClient,
    plan: &SwitchPlan,
    _authorization: &OfflineAuthorization,
) -> Result<(), ExecuteError> {
    match &plan.restore {
        Restore::Stop => {
            client.call(SERVICE, PATH, INTERFACE, "StopScheduler", "", &[])?;
            let state = read_state(client)?;
            if state.current.is_some() {
                return Err(ExecuteError::NotInEffect {
                    wanted: "no scheduler".into(),
                    current: state.current,
                });
            }
        }
        Restore::SwitchBack { name, mode } => {
            call_switch(client, "SwitchScheduler", name, *mode)?;
            let state = read_state(client)?;
            if state.current.as_deref() != Some(name.as_str()) || state.mode != Some(*mode) {
                return Err(ExecuteError::NotInEffect {
                    wanted: name.clone(),
                    current: state.current,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "scx_loader_tests.rs"]
mod tests;
