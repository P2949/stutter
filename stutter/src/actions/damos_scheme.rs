//! A Stutter-owned DAMOS scheme as a prepared transaction (package W11c).
//!
//! Claims one idle kdamond (state `off`, no contexts), configures one `vaddr` context for one
//! verified process with one scheme, starts it, and on restore stops and clears exactly that
//! configuration. Configuration restores exactly; **page reclaim or migration does not**, so
//! every action except `stat` is classed irreversible, requires an explicit acknowledgement and a
//! bounded quota. Experimental and never autonomous; absent or root-only sysfs is `Unavailable`.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamosAction {
    /// Monitoring only; nothing is reclaimed or moved.
    Stat,
    Pageout,
    LruDeprio,
    LruPrio,
}

impl DamosAction {
    fn name(self) -> &'static str {
        match self {
            Self::Stat => "stat",
            Self::Pageout => "pageout",
            Self::LruDeprio => "lru_deprio",
            Self::LruPrio => "lru_prio",
        }
    }

    /// Whether the action changes memory state that restoring the configuration cannot undo.
    pub fn irreversible(self) -> bool {
        self != Self::Stat
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamosQuota {
    /// Milliseconds of CPU per reset interval.
    pub ms: u64,
    pub bytes: u64,
    pub reset_interval_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamosRequest {
    /// The one process whose address space is monitored, with its live start time.
    pub task: u32,
    pub start_ticks: u64,
    pub action: DamosAction,
    pub quota: DamosQuota,
    /// Required for any irreversible action.
    pub acknowledge_irreversible: bool,
    /// Region size bounds, bytes.
    pub min_region_bytes: u64,
    pub max_region_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamosDenial {
    Unavailable,
    /// The target is not the live process that was named.
    TargetIdentity,
    IrreversibleNotAcknowledged,
    /// An irreversible action needs a nonzero, bounded quota.
    QuotaUnbounded,
    BadRegion,
    /// No kdamond is idle: someone else's configuration is never touched.
    NoIdleKdamond,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamosCommitError {
    Write,
    /// The kernel did not report the configuration or the run state; it was torn down.
    ReadbackMismatch {
        cleared: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamosRestore {
    /// Stopped and cleared; configuration is exact, any reclaim or migration stays done.
    Cleared {
        irreversible_effect_possible: bool,
    },
    /// The kdamond no longer holds our configuration: left untouched.
    ExternallyChanged,
    Unproven,
}

#[derive(Debug)]
pub struct DamosPrepared {
    kdamond: PathBuf,
    request: DamosRequest,
}

#[derive(Debug)]
pub struct DamosCommitted {
    kdamond: PathBuf,
    request: DamosRequest,
}

fn text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|t| t.trim().to_owned())
}

fn put(path: &Path, value: impl ToString) -> io::Result<()> {
    fs::write(path, value.to_string())
}

/// Validates the request and finds an idle kdamond; writes nothing. `live_start_ticks` proves the
/// target is the process that was named.
pub fn prepare(
    admin: &Path,
    request: &DamosRequest,
    live_start_ticks: impl Fn(u32) -> Option<u64>,
) -> Result<DamosPrepared, DamosDenial> {
    let kdamonds = admin.join("kdamonds");
    let count: usize = text(&kdamonds.join("nr_kdamonds"))
        .and_then(|t| t.parse().ok())
        .ok_or(DamosDenial::Unavailable)?;
    if live_start_ticks(request.task) != Some(request.start_ticks) {
        return Err(DamosDenial::TargetIdentity);
    }
    if request.action.irreversible() {
        if !request.acknowledge_irreversible {
            return Err(DamosDenial::IrreversibleNotAcknowledged);
        }
        let quota = request.quota;
        if quota.ms == 0 || quota.ms > 10 || quota.bytes == 0 || quota.reset_interval_ms < 100 {
            return Err(DamosDenial::QuotaUnbounded);
        }
    }
    if request.min_region_bytes == 0 || request.min_region_bytes > request.max_region_bytes {
        return Err(DamosDenial::BadRegion);
    }
    let kdamond = (0..count)
        .map(|index| kdamonds.join(index.to_string()))
        .find(|dir| {
            text(&dir.join("state")).as_deref() == Some("off")
                && text(&dir.join("contexts/nr_contexts")).as_deref() == Some("0")
        })
        .ok_or(DamosDenial::NoIdleKdamond)?;
    Ok(DamosPrepared {
        kdamond,
        request: request.clone(),
    })
}

impl DamosPrepared {
    pub fn commit(self) -> Result<DamosCommitted, DamosCommitError> {
        let r = &self.request;
        let context = self.kdamond.join("contexts/0");
        let scheme = context.join("schemes/0");
        let configure = || -> io::Result<()> {
            put(&self.kdamond.join("contexts/nr_contexts"), 1)?;
            put(&context.join("operations"), "vaddr")?;
            put(&context.join("targets/nr_targets"), 1)?;
            put(&context.join("targets/0/pid_target"), r.task)?;
            put(&context.join("schemes/nr_schemes"), 1)?;
            put(&scheme.join("action"), r.action.name())?;
            put(&scheme.join("quotas/ms"), r.quota.ms)?;
            put(&scheme.join("quotas/bytes"), r.quota.bytes)?;
            put(
                &scheme.join("quotas/reset_interval_ms"),
                r.quota.reset_interval_ms,
            )?;
            put(&scheme.join("access_pattern/sz/min"), r.min_region_bytes)?;
            put(&scheme.join("access_pattern/sz/max"), r.max_region_bytes)?;
            put(&self.kdamond.join("state"), "on")
        };
        let committed = DamosCommitted {
            kdamond: self.kdamond.clone(),
            request: self.request.clone(),
        };
        if configure().is_err() {
            // Nothing is running unless `state` was reached; clear whatever was written.
            let _ = committed.clear();
            return Err(DamosCommitError::Write);
        }
        if !committed.holds_ours() {
            let cleared = committed.clear();
            return Err(DamosCommitError::ReadbackMismatch { cleared });
        }
        Ok(committed)
    }
}

impl DamosCommitted {
    /// The kdamond is running our one context, target and scheme.
    fn holds_ours(&self) -> bool {
        let context = self.kdamond.join("contexts/0");
        let scheme = context.join("schemes/0");
        text(&self.kdamond.join("state")).as_deref() == Some("on")
            && text(&self.kdamond.join("contexts/nr_contexts")).as_deref() == Some("1")
            && text(&context.join("targets/0/pid_target")) == Some(self.request.task.to_string())
            && text(&scheme.join("action")).as_deref() == Some(self.request.action.name())
    }

    fn clear(&self) -> bool {
        let context = self.kdamond.join("contexts/0");
        put(&self.kdamond.join("state"), "off").is_ok()
            && put(&context.join("schemes/nr_schemes"), 0).is_ok()
            && put(&context.join("targets/nr_targets"), 0).is_ok()
            && put(&self.kdamond.join("contexts/nr_contexts"), 0).is_ok()
            && text(&self.kdamond.join("state")).as_deref() == Some("off")
            && text(&self.kdamond.join("contexts/nr_contexts")).as_deref() == Some("0")
    }

    /// Stops and clears the configuration only while the kdamond still holds exactly ours.
    pub fn restore(self) -> DamosRestore {
        if !self.holds_ours() {
            return DamosRestore::ExternallyChanged;
        }
        if self.clear() {
            DamosRestore::Cleared {
                irreversible_effect_possible: self.request.action.irreversible(),
            }
        } else {
            DamosRestore::Unproven
        }
    }
}

#[cfg(test)]
#[path = "damos_scheme_tests.rs"]
mod tests;
