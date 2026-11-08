//! Read-only Gamescope capability probe.
//!
//! Stutter never starts Gamescope or changes presentation settings to improve a measurement. It
//! only asks an installed binary what it supports, so `--mangoapp` is used strictly when the caller
//! already chose Gamescope *and* this build of it accepts the flag. Without it, display-path
//! context comes from the existing Wayland, KMS and DRM probes, never faux application frames.
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// Longest the help probe may run before it is killed.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Most help text read, in bytes.
const MAX_HELP_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GamescopeSupport {
    /// No usable binary.
    NotInstalled,
    /// The binary did not answer in time or could not be run.
    Unprobeable(String),
    /// Installed; `mangoapp` says whether `--mangoapp` is documented by this build.
    Installed { mangoapp: bool },
}

/// Whether `--mangoapp` appears as a documented option in `help`.
pub fn help_documents_mangoapp(help: &str) -> bool {
    help.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("--mangoapp") || line.contains(" --mangoapp")
    })
}

/// Runs `<binary> --help` under a deadline and reads what this build supports.
pub fn probe(binary: &Path) -> GamescopeSupport {
    if !binary.is_file() {
        return GamescopeSupport::NotInstalled;
    }
    let mut child = match Command::new(binary)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return GamescopeSupport::Unprobeable(error.to_string()),
    };
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        for stream in [
            stdout.as_mut().map(|s| s as &mut dyn Read),
            stderr.as_mut().map(|s| s as &mut dyn Read),
        ]
        .into_iter()
        .flatten()
        {
            let mut part = String::new();
            let _ = stream.take(MAX_HELP_BYTES).read_to_string(&mut part);
            text.push_str(&part);
        }
        text
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return GamescopeSupport::Unprobeable("--help did not finish in time".into());
            }
            Err(error) => return GamescopeSupport::Unprobeable(error.to_string()),
        }
    }
    match reader.join() {
        Ok(help) => GamescopeSupport::Installed {
            mangoapp: help_documents_mangoapp(&help),
        },
        Err(_) => GamescopeSupport::Unprobeable("help reader failed".into()),
    }
}
