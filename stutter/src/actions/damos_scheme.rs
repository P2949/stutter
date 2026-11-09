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

/// What a quota goal tracks. `UserInput` is fed by Stutter through [`feed_goal`]; the kernel
/// measures `SomeMemPsiUs` itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamosGoalMetric {
    UserInput,
    SomeMemPsiUs,
}

impl DamosGoalMetric {
    fn name(self) -> &'static str {
        match self {
            Self::UserInput => "user_input",
            Self::SomeMemPsiUs => "some_mem_psi_us",
        }
    }
}

/// A quota goal: the kernel tunes the scheme's effective quota toward `target_value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamosGoal {
    pub metric: DamosGoalMetric,
    pub target_value: u64,
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
    /// Optional quota goal feedback; needs a nonzero quota to steer.
    pub goal: Option<DamosGoal>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamosDenial {
    /// A goal needs a nonzero quota and a nonzero target.
    BadGoal,
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
    /// Stopped and cleared, but the watched pid now names a different process than the one that
    /// was recorded (the original exited and the number was reused). The kdamond holds a reference
    /// to the original process, not the number, so nothing else was monitored or touched.
    ClearedTargetReplaced {
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

/// What recovery needs to stop and clear exactly the configuration this transaction made: the
/// complete configuration it wrote, the identity of the process it watches, and the identity of
/// the kernel object (the kdamond's sysfs directory and its worker thread) that carries it. A
/// record that does not match every part is somebody else's and is never touched. Records from
/// before these fields existed deserialize to zeros and so match nothing: manual attention.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DamosRecord {
    pub kdamond: PathBuf,
    pub task: u32,
    pub action: String,
    pub irreversible: bool,
    /// The watched process's start time (clock ticks since boot) when it was committed.
    #[serde(default)]
    pub task_start_ticks: u64,
    #[serde(default)]
    pub quota_ms: u64,
    #[serde(default)]
    pub quota_bytes: u64,
    #[serde(default)]
    pub quota_reset_interval_ms: u64,
    #[serde(default)]
    pub min_region_bytes: u64,
    #[serde(default)]
    pub max_region_bytes: u64,
    /// Device and inode of the kdamond's sysfs directory.
    #[serde(default)]
    pub kdamond_dev: u64,
    #[serde(default)]
    pub kdamond_ino: u64,
    /// The kdamond worker thread's pid, as the kernel reports it while running.
    #[serde(default)]
    pub kdamond_pid: Option<String>,
    /// The goal metric written, if any, and its target.
    #[serde(default)]
    pub goal_metric: Option<String>,
    #[serde(default)]
    pub goal_target: u64,
}

#[derive(Debug)]
pub struct DamosCommitted {
    record: DamosRecord,
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
    if let Some(goal) = request.goal
        && (goal.target_value == 0 || (request.quota.ms == 0 && request.quota.bytes == 0))
    {
        return Err(DamosDenial::BadGoal);
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
            if let Some(goal) = r.goal {
                let goals = scheme.join("quotas/goals");
                put(&goals.join("nr_goals"), 1)?;
                put(&goals.join("0/target_metric"), goal.metric.name())?;
                put(&goals.join("0/target_value"), goal.target_value)?;
            }
            put(&self.kdamond.join("state"), "on")
        };
        if configure().is_err() {
            // Nothing is running unless `state` was reached; clear whatever was written.
            let _ = stop_and_clear(&self.kdamond);
            return Err(DamosCommitError::Write);
        }
        // The record is taken from what the kernel reports after the start, never from the
        // request alone, so it names the object that is actually running.
        let Some((dev, ino, pid)) = kdamond_identity(&self.kdamond) else {
            let cleared = stop_and_clear(&self.kdamond);
            return Err(DamosCommitError::ReadbackMismatch { cleared });
        };
        let record = DamosRecord {
            kdamond: self.kdamond.clone(),
            task: r.task,
            action: r.action.name().to_owned(),
            irreversible: r.action.irreversible(),
            task_start_ticks: r.start_ticks,
            quota_ms: r.quota.ms,
            quota_bytes: r.quota.bytes,
            quota_reset_interval_ms: r.quota.reset_interval_ms,
            min_region_bytes: r.min_region_bytes,
            max_region_bytes: r.max_region_bytes,
            kdamond_dev: dev,
            kdamond_ino: ino,
            kdamond_pid: pid,
            goal_metric: r.goal.map(|goal| goal.metric.name().to_owned()),
            goal_target: r.goal.map_or(0, |goal| goal.target_value),
        };
        if !holds(&self.kdamond, &record) {
            let cleared = stop_and_clear(&self.kdamond);
            return Err(DamosCommitError::ReadbackMismatch { cleared });
        }
        Ok(DamosCommitted { record })
    }
}

impl DamosCommitted {
    pub fn record(&self) -> DamosRecord {
        self.record.clone()
    }

    /// Stops and clears the configuration only while the kdamond still holds exactly ours.
    pub fn restore(self) -> DamosRestore {
        teardown_recorded(&self.record)
    }
}

/// The kdamond's own identity: its sysfs object and worker pid.
fn kdamond_identity(kdamond: &Path) -> Option<(u64, u64, Option<String>)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(kdamond).ok()?;
    Some((meta.dev(), meta.ino(), text(&kdamond.join("pid"))))
}

/// True only while the kdamond runs exactly the one context, target and scheme described, with
/// every value this transaction wrote, and is the same kernel object that was recorded.
fn holds(kdamond: &Path, record: &DamosRecord) -> bool {
    let context = kdamond.join("contexts/0");
    let scheme = context.join("schemes/0");
    let value = |path: PathBuf| text(&path);
    let number = |path: PathBuf, want: u64| value(path) == Some(want.to_string());
    kdamond_identity(kdamond)
        == Some((
            record.kdamond_dev,
            record.kdamond_ino,
            record.kdamond_pid.clone(),
        ))
        && value(kdamond.join("state")).as_deref() == Some("on")
        && value(kdamond.join("contexts/nr_contexts")).as_deref() == Some("1")
        && value(context.join("operations")).as_deref() == Some("vaddr")
        && value(context.join("targets/nr_targets")).as_deref() == Some("1")
        && number(context.join("targets/0/pid_target"), u64::from(record.task))
        && value(scheme.join("action")).as_deref() == Some(record.action.as_str())
        && value(context.join("schemes/nr_schemes")).as_deref() == Some("1")
        && number(scheme.join("quotas/ms"), record.quota_ms)
        && number(scheme.join("quotas/bytes"), record.quota_bytes)
        && number(
            scheme.join("quotas/reset_interval_ms"),
            record.quota_reset_interval_ms,
        )
        && number(
            scheme.join("access_pattern/sz/min"),
            record.min_region_bytes,
        )
        && number(
            scheme.join("access_pattern/sz/max"),
            record.max_region_bytes,
        )
        && goal_holds(&scheme, record)
}

/// The scheme carries exactly the recorded goal, or no goal when none was recorded.
fn goal_holds(scheme: &Path, record: &DamosRecord) -> bool {
    let goals = scheme.join("quotas/goals");
    let count = text(&goals.join("nr_goals"));
    match &record.goal_metric {
        None => count.is_none() || count.as_deref() == Some("0"),
        Some(metric) => {
            count.as_deref() == Some("1")
                && text(&goals.join("0/target_metric")).as_deref() == Some(metric.as_str())
                && text(&goals.join("0/target_value")) == Some(record.goal_target.to_string())
        }
    }
}

/// Whether the process a scheme was started for is still the one that was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetState {
    Same,
    /// The process exited. Nothing is left to monitor, and clearing is still correct.
    Gone,
    /// The pid is alive with a different start time: a reused number, not our target.
    Replaced,
}

fn target_state(
    record: &DamosRecord,
    live_start_ticks: &dyn Fn(u32) -> Option<u64>,
) -> TargetState {
    match live_start_ticks(record.task) {
        Some(ticks) if ticks == record.task_start_ticks => TargetState::Same,
        Some(_) => TargetState::Replaced,
        None => TargetState::Gone,
    }
}

fn live_process_start(task: u32) -> Option<u64> {
    crate::telemetry::proc_view::live_process(Path::new("/proc"), task).map(|live| live.start_ticks)
}

#[derive(Debug, PartialEq, Eq)]
pub enum DamosFeedbackError {
    /// The scheme has no `user_input` goal to feed.
    NoUserInputGoal,
    /// The target exited, so there is nothing to feed a measurement for.
    TargetGone,
    /// The recorded pid now names a different process; its measurements are not ours to feed.
    TargetReplaced,
    /// The kdamond is not the recorded object holding the recorded configuration.
    NotOurs,
    Write,
}

/// Feeds the measured value of a `user_input` goal to the kernel, only while the kdamond is still
/// exactly the recorded one; the value written must read back.
pub fn feed_goal(record: &DamosRecord, current: u64) -> Result<(), DamosFeedbackError> {
    feed_goal_with(record, current, &live_process_start)
}

/// [`feed_goal`] with the live start-time lookup supplied (the host's `/proc` in production).
pub fn feed_goal_with(
    record: &DamosRecord,
    current: u64,
    live_start_ticks: &dyn Fn(u32) -> Option<u64>,
) -> Result<(), DamosFeedbackError> {
    if record.goal_metric.as_deref() != Some(DamosGoalMetric::UserInput.name()) {
        return Err(DamosFeedbackError::NoUserInputGoal);
    }
    match target_state(record, live_start_ticks) {
        TargetState::Same => {}
        TargetState::Gone => return Err(DamosFeedbackError::TargetGone),
        TargetState::Replaced => return Err(DamosFeedbackError::TargetReplaced),
    }
    if !holds(&record.kdamond, record) {
        return Err(DamosFeedbackError::NotOurs);
    }
    let path = record
        .kdamond
        .join("contexts/0/schemes/0/quotas/goals/0/current_value");
    put(&path, current).map_err(|_| DamosFeedbackError::Write)?;
    if text(&path) == Some(current.to_string()) {
        Ok(())
    } else {
        Err(DamosFeedbackError::Write)
    }
}

fn stop_and_clear(kdamond: &Path) -> bool {
    let context = kdamond.join("contexts/0");
    put(&kdamond.join("state"), "off").is_ok()
        && put(&context.join("schemes/nr_schemes"), 0).is_ok()
        && put(&context.join("targets/nr_targets"), 0).is_ok()
        && put(&kdamond.join("contexts/nr_contexts"), 0).is_ok()
        && text(&kdamond.join("state")).as_deref() == Some("off")
        && text(&kdamond.join("contexts/nr_contexts")).as_deref() == Some("0")
}

/// Stops and clears the recorded kdamond only while it is the recorded kernel object and still
/// holds every recorded value; anything else is left untouched.
pub fn teardown_recorded(record: &DamosRecord) -> DamosRestore {
    teardown_recorded_with(record, &live_process_start)
}

/// [`teardown_recorded`] with the live start-time lookup supplied. A target that exited is cleared
/// as before; a pid that now names a different process is cleared too but reported as such, so
/// recovery never mistakes a reused number for the recorded process.
pub fn teardown_recorded_with(
    record: &DamosRecord,
    live_start_ticks: &dyn Fn(u32) -> Option<u64>,
) -> DamosRestore {
    let target = target_state(record, live_start_ticks);
    let cleared = |irreversible_effect_possible| match target {
        TargetState::Replaced => DamosRestore::ClearedTargetReplaced {
            irreversible_effect_possible,
        },
        _ => DamosRestore::Cleared {
            irreversible_effect_possible,
        },
    };
    if !holds(&record.kdamond, record) {
        // Already torn down is success, but only if the same kdamond object is idle and empty; a
        // replaced or reconfigured one is not ours and is left alone.
        let idle = text(&record.kdamond.join("state")).as_deref() == Some("off")
            && text(&record.kdamond.join("contexts/nr_contexts")).as_deref() == Some("0")
            && kdamond_identity(&record.kdamond).is_some_and(|(dev, ino, _)| {
                (dev, ino) == (record.kdamond_dev, record.kdamond_ino)
            });
        return if idle {
            cleared(record.irreversible)
        } else {
            DamosRestore::ExternallyChanged
        };
    }
    if stop_and_clear(&record.kdamond) {
        cleared(record.irreversible)
    } else {
        DamosRestore::Unproven
    }
}

#[cfg(test)]
#[path = "damos_scheme_tests.rs"]
mod tests;
