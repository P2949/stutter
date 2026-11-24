//! The production composition of the CP-LONG route driver. The operator's session file names the route, the key script,
//! the save the session starts from, the helper's socket inside the isolated session and the live session's own
//! identifiers; this module turns it into a `RouteDriver` and nothing else. Nothing is sent to the game until the host has
//! attested the game process and the input has proved where it sends:
//!
//! * the session file must be a regular file owned by the controller's user, writable by no one else, and small;
//! * the save the session starts from is part of the definition a work unit must carry, hashed anew at every start;
//! * when the game process is attested, its own environment (read from the kernel, for that very pid and start time)
//!   must show a display, session and logind seat of its own (`isolated_seat::attest`), and only then is the helper's
//!   private socket connected, naming that display, through `IsolatedSink`, which refuses a sink bound anywhere else.
//!
//! Any refusal fails the session with a typed reason before the first key. Without the session file the build keeps
//! `UnavailableInput`. Nothing here creates an input device: the helper that does lives inside the isolated session.

use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{RouteDriver, RouteInput, RouteStep};
use crate::autotune::research::{
    long_receipt::SessionProcess,
    scripted_input::{
        Key, KeyStep, PhaseScript, ScriptedInput,
        isolated_seat::{IsolatedSink, LiveSession, attest},
        script_sha256_of,
        seat_socket_sink::SeatSocketSink,
    },
};

/// The environment variable naming the session file.
pub const ENV: &str = "STUTTER_RESEARCH_LONG_SESSION";
const MAX_FILE_BYTES: u64 = 1 << 20;
const MAX_ENVIRON_BYTES: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepFile {
    phase: String,
    seconds: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    at_ms: u64,
    key: String,
    down: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseFile {
    phase: String,
    steps: Vec<KeyFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveFile {
    wayland_display: String,
    x_display: String,
    session_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFile {
    route: Vec<StepFile>,
    script: Vec<PhaseFile>,
    save_dir: PathBuf,
    helper_socket: PathBuf,
    live: LiveFile,
}

fn key_named(name: &str) -> Option<Key> {
    Some(match name.to_ascii_lowercase().as_str() {
        "w" => Key::W,
        "a" => Key::A,
        "s" => Key::S,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "enter" => Key::Enter,
        "escape" => Key::Escape,
        "leftshift" => Key::LeftShift,
        _ => return None,
    })
}

/// What a valid session file gives: the input, the route and the save directory.
pub struct Loaded {
    pub input: SessionInput,
    pub route: Vec<RouteStep>,
    pub save_dir: PathBuf,
}

/// Read and validate the session file. It must be the controller user's own private regular file; every refusal is typed.
pub fn load(path: &Path, uid: u32) -> Result<Loaded, &'static str> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| "LongSessionConfigUnreadable")?;
    let meta = file.metadata().map_err(|_| "LongSessionConfigUnreadable")?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("LongSessionConfigUnreadable");
    }
    if meta.uid() != uid || meta.mode() & 0o022 != 0 {
        return Err("LongSessionConfigNotPrivate");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| "LongSessionConfigUnreadable")?;
    let parsed: SessionFile =
        serde_json::from_slice(&bytes).map_err(|_| "LongSessionConfigInvalid")?;
    let route: Vec<RouteStep> = parsed
        .route
        .into_iter()
        .map(|step| RouteStep {
            phase: step.phase,
            seconds: step.seconds,
        })
        .collect();
    let mut script = Vec::new();
    for phase in parsed.script {
        let mut steps = Vec::new();
        for step in phase.steps {
            steps.push(KeyStep {
                at_ms: step.at_ms,
                key: key_named(&step.key).ok_or("LongSessionConfigInvalid")?,
                down: step.down,
            });
        }
        script.push(PhaseScript {
            phase: phase.phase,
            steps,
        });
    }
    // The route must be non-empty, every step positive and named, and every phase of it must have a script.
    if route.is_empty()
        || route
            .iter()
            .any(|step| step.phase.is_empty() || step.seconds == 0)
        || route
            .iter()
            .any(|step| !script.iter().any(|s| s.phase == step.phase))
        || !parsed.save_dir.is_absolute()
        || !parsed.helper_socket.is_absolute()
    {
        return Err("LongSessionConfigInvalid");
    }
    let live = LiveSession {
        wayland_display: parsed.live.wayland_display,
        x_display: parsed.live.x_display,
        session_id: parsed.live.session_id,
    };
    Ok(Loaded {
        input: SessionInput {
            script_sha256: script_sha256_of(&script),
            script,
            helper_socket: parsed.helper_socket,
            live,
            uid,
            inner: None,
        },
        route,
        save_dir: parsed.save_dir,
    })
}

/// The route driver the session file describes, on the system clock.
pub fn driver_from(path: &Path, uid: u32) -> Result<RouteDriver<SessionInput>, &'static str> {
    let loaded = load(path, uid)?;
    Ok(RouteDriver::new(
        loaded.input,
        loaded.route,
        Box::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs())
        }),
    )
    .with_save_dir(loaded.save_dir))
}

/// The route's input: bound to the attested game when the session starts, and refusing everything before then.
pub struct SessionInput {
    script: Vec<PhaseScript>,
    script_sha256: String,
    helper_socket: PathBuf,
    live: LiveSession,
    uid: u32,
    inner: Option<ScriptedInput<IsolatedSink<SeatSocketSink>>>,
}

/// The game's own environment, read from the kernel for exactly this process: the pid must still be the process that was
/// attested (same start time), or the environment is some other process's.
fn game_environment(process: SessionProcess) -> Result<Vec<u8>, &'static str> {
    let pid = process.pid.as_u32();
    if crate::process_tree::process_starttime_at(Path::new("/proc"), pid)
        != Some(process.start_ticks)
    {
        return Err("GameProcessChanged");
    }
    let mut bytes = Vec::new();
    std::fs::File::open(format!("/proc/{pid}/environ"))
        .and_then(|file| file.take(MAX_ENVIRON_BYTES).read_to_end(&mut bytes))
        .map_err(|_| "GameEnvironmentUnreadable")?;
    // And it is still that process after the read.
    if crate::process_tree::process_starttime_at(Path::new("/proc"), pid)
        != Some(process.start_ticks)
    {
        return Err("GameProcessChanged");
    }
    Ok(bytes)
}

impl RouteInput for SessionInput {
    fn available(&self) -> bool {
        true
    }

    fn bind(&mut self, process: SessionProcess) -> Result<(), &'static str> {
        let environ = game_environment(process)?;
        // The seat is the game session's own and never the live one; nothing else is attested.
        let seat = attest(&environ, &self.live)?;
        let sink = SeatSocketSink::connect(&self.helper_socket, self.uid, seat.display())?;
        let sink = IsolatedSink::new(sink, &seat)?;
        self.inner = Some(ScriptedInput::new(sink, self.script.clone()));
        Ok(())
    }

    fn script_sha256(&self) -> String {
        self.script_sha256.clone()
    }

    fn begin_phase(&mut self, phase: &str) -> Result<(), &'static str> {
        self.inner
            .as_mut()
            .ok_or("LongInputNotBound")?
            .begin_phase(phase)
    }

    fn tick(&mut self, elapsed_ms: u64) -> Result<(), &'static str> {
        self.inner
            .as_mut()
            .ok_or("LongInputNotBound")?
            .tick(elapsed_ms)
    }

    fn stop(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.stop();
        }
    }

    fn released(&self) -> bool {
        self.inner.as_ref().is_none_or(RouteInput::released)
    }
}

#[cfg(test)]
#[path = "long_session_input_tests.rs"]
mod tests;
