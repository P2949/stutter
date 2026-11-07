//! `stutter game-run -- <argv...>`: an optional, unprivileged launch wrapper.
//!
//! It starts the exact argv with its working directory, environment, stdio, exit status and
//! signals preserved, and optionally points MangoHud at a secure, unique capture directory. It
//! never forces itself or MangoHud onto an existing launch, holds no privilege, and a launch id
//! it records is correlation data only. Whether MangoHud actually logs is verified later by the
//! tail session (a recognized header must appear), never assumed from the configuration.
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::telemetry::{
    identity::{new_capture_id, read_boot_id},
    proc_view::live_process,
    session::ConfinedDir,
};

/// Capture directories kept per root; older ones this tool created are removed.
pub const KEEP_CAPTURES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub launch_id: String,
    pub boot_id: String,
    pub uid: u32,
    pub pid: u32,
    pub start_ticks: u64,
    pub argv: Vec<String>,
    pub mangohud_requested: bool,
}

#[derive(Clone, Debug)]
pub struct GameRunPlan {
    pub argv: Vec<OsString>,
    pub capture_root: PathBuf,
    pub mangohud: bool,
}

/// `$XDG_RUNTIME_DIR/stutter/game-run`.
pub fn default_capture_root() -> anyhow::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .context("XDG_RUNTIME_DIR is not set; pass --capture-root")?;
    Ok(runtime.join("stutter").join("game-run"))
}

fn is_capture_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Creates `root` (owner-only) if needed and a fresh unique capture directory inside it.
pub fn create_capture_dir(root: &Path, uid: u32) -> anyhow::Result<(String, PathBuf)> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(root)
        .with_context(|| format!("cannot create {}", root.display()))?;
    ConfinedDir::open(root, uid)
        .map_err(|error| anyhow::anyhow!("capture root {} is unsafe: {error:?}", root.display()))?;
    let capture_id = new_capture_id()?;
    let dir = root.join(&capture_id);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    Ok((capture_id, dir))
}

/// Removes the oldest capture directories beyond `keep`. Only real directories with a capture
/// name and this owner are touched; a symlink or anything else is left alone.
pub fn prune_captures(root: &Path, uid: u32, keep: usize) -> std::io::Result<usize> {
    let mut captures = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if is_capture_name(&name) && meta.file_type().is_dir() && meta.uid() == uid {
            captures.push((meta.mtime(), meta.mtime_nsec(), entry.path()));
        }
    }
    captures.sort();
    let excess = captures.len().saturating_sub(keep);
    for (_, _, path) in captures.iter().take(excess) {
        fs::remove_dir_all(path)?;
    }
    Ok(excess)
}

/// `existing` MangoHud configuration plus the capture directory and a log that starts at once.
pub fn mangohud_config(existing: Option<&str>, capture_dir: &Path) -> anyhow::Result<String> {
    let dir = capture_dir
        .to_str()
        .filter(|dir| !dir.contains(',') && !dir.contains('\n'))
        .context("capture directory path cannot be placed in a MangoHud config")?;
    let mut config = existing
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("{value},"))
        .unwrap_or_default();
    config.push_str(&format!("output_folder={dir},autostart_log=1"));
    Ok(config)
}

/// The child command: exact argv, inherited cwd, stdio and environment, plus correlation
/// variables, and the MangoHud variables only when the caller opted in.
pub fn build_command(
    plan: &GameRunPlan,
    capture_dir: &Path,
    launch_id: &str,
) -> anyhow::Result<Command> {
    let (program, args) = plan
        .argv
        .split_first()
        .context("game-run needs a command after `--`")?;
    let mut command = Command::new(program);
    command.args(args);
    command.env("STUTTER_LAUNCH_ID", launch_id);
    command.env("STUTTER_CAPTURE_DIR", capture_dir);
    if plan.mangohud {
        let existing = std::env::var("MANGOHUD_CONFIG").ok();
        command.env("MANGOHUD", "1");
        command.env(
            "MANGOHUD_CONFIG",
            mangohud_config(existing.as_deref(), capture_dir)?,
        );
    }
    Ok(command)
}

fn write_record(dir: &Path, record: &LaunchRecord) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("launch.json"))?;
    file.write_all(serde_json::to_string_pretty(record)?.as_bytes())?;
    Ok(())
}

/// Reads and validates the launch record of a capture directory.
pub fn read_launch_record(dir: &Path, uid: u32) -> anyhow::Result<LaunchRecord> {
    let confined = ConfinedDir::open(dir, uid)
        .map_err(|error| anyhow::anyhow!("capture directory is unsafe: {error:?}"))?;
    let (mut file, _) = confined
        .open_file("launch.json")
        .map_err(|error| anyhow::anyhow!("launch record is unsafe: {error:?}"))?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file, &mut text)?;
    Ok(serde_json::from_str(&text)?)
}

/// Runs the plan and returns the exit code the wrapper itself should exit with: the child's own
/// code, or `128 + signal` when a signal ended it.
pub fn run(plan: &GameRunPlan, forward_signals: bool) -> anyhow::Result<i32> {
    let uid = crate::syscall::geteuid();
    let (capture_id, dir) = create_capture_dir(&plan.capture_root, uid)?;
    let launch_id = capture_id;
    let mut command = build_command(plan, &dir, &launch_id)?;
    if forward_signals {
        // Process-global, so only the command-line entry point opts in.
        crate::syscall::catch_termination_signals();
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start {:?}", plan.argv[0]))?;
    let pid = child.id();
    if let (Ok(boot_id), Some(live)) = (read_boot_id(), live_process(Path::new("/proc"), pid)) {
        let record = LaunchRecord {
            launch_id,
            boot_id,
            uid,
            pid,
            start_ticks: live.start_ticks,
            argv: plan
                .argv
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            mangohud_requested: plan.mangohud,
        };
        if let Err(error) = write_record(&dir, &record) {
            log::warn!("game_run_launch_record_failed err={error:#}");
        }
    }
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Some(signal) = forward_signals
            .then(crate::syscall::take_caught_signal)
            .flatten()
        {
            let _ = crate::syscall::send_signal(pid, signal);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if let Err(error) = prune_captures(&plan.capture_root, uid, KEEP_CAPTURES) {
        log::warn!("game_run_prune_failed err={error}");
    }
    match (status.code(), status.signal()) {
        (Some(code), _) => Ok(code),
        (None, Some(signal)) => Ok(128 + signal),
        (None, None) => bail!("child ended without a status"),
    }
}

#[cfg(test)]
mod tests;
