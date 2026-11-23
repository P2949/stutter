//! The scripted `RouteInput`: it turns a route's per-phase key script into Linux input events, written
//! through an `EventSink`. The script, the timing, the event encoding and the guarantee that no key is ever
//! left held are all real and tested here; the sink is the only part that touches a device.
//!
//! No device sink exists in this repository on purpose. A virtual keyboard registered on the user's machine
//! is picked up by the running compositor and delivers its keys to whatever window is focused, so a backend
//! that writes to `/dev/uinput` can only be exercised in a session of its own. Until one is supplied the
//! build keeps `UnavailableInput`, and nothing here can reach a desktop.

use super::route_driver::RouteInput;

/// The gate a device sink must pass before it may reach the game (staged with this module: no sink exists).
#[path = "isolated_seat.rs"]
pub(crate) mod isolated_seat;
/// The helper inside the isolated session: decoded events to an injector, every key released at the end.
#[path = "seat_helper.rs"]
pub(crate) mod seat_helper;
/// The transport half of a sink: events to a helper inside the isolated session.
#[path = "seat_socket_sink.rs"]
pub(crate) mod seat_socket_sink;

/// The keys a route may press (Linux `KEY_*` codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    W,
    A,
    S,
    D,
    E,
    F,
    Tab,
    Space,
    Enter,
    Escape,
    LeftShift,
}

impl Key {
    pub const ALL: [Key; 11] = [
        Self::W,
        Self::A,
        Self::S,
        Self::D,
        Self::E,
        Self::F,
        Self::Tab,
        Self::Space,
        Self::Enter,
        Self::Escape,
        Self::LeftShift,
    ];

    /// The `linux/input-event-codes.h` code.
    pub fn code(self) -> u16 {
        match self {
            Self::W => 17,
            Self::A => 30,
            Self::S => 31,
            Self::D => 32,
            Self::E => 18,
            Self::F => 33,
            Self::Tab => 15,
            Self::Space => 57,
            Self::Enter => 28,
            Self::Escape => 1,
            Self::LeftShift => 42,
        }
    }
}

/// One scheduled key transition, relative to the start of its phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyStep {
    pub at_ms: u64,
    pub key: Key,
    pub down: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseScript {
    pub phase: String,
    pub steps: Vec<KeyStep>,
}

/// Where encoded events go. Each call carries whole `struct input_event` records.
pub trait EventSink: Send {
    fn write(&mut self, events: &[u8]) -> Result<(), &'static str>;
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
/// `struct input_event` on 64-bit little-endian Linux: timeval (two i64), type, code, value.
pub const EVENT_BYTES: usize = 24;
/// A key step more than this late was not delivered when the route said, so the session fails rather than
/// credit a phase whose inputs arrived out of time.
pub const MAX_STEP_LATENESS_MS: u64 = 2_000;

const _: () = assert!(std::mem::size_of::<usize>() == 8 && cfg!(target_endian = "little"));

/// A key transition followed by the synchronization report that makes it take effect.
pub fn encode_key(key: Key, down: bool) -> [u8; 2 * EVENT_BYTES] {
    let mut out = [0u8; 2 * EVENT_BYTES];
    let mut put = |index: usize, kind: u16, code: u16, value: i32| {
        let at = index * EVENT_BYTES;
        // The kernel stamps the time itself, so the timeval stays zero.
        out[at + 16..at + 18].copy_from_slice(&kind.to_le_bytes());
        out[at + 18..at + 20].copy_from_slice(&code.to_le_bytes());
        out[at + 20..at + 24].copy_from_slice(&value.to_le_bytes());
    };
    put(0, EV_KEY, key.code(), i32::from(down));
    put(1, EV_SYN, SYN_REPORT, 0);
    out
}

pub struct ScriptedInput<S: EventSink> {
    sink: S,
    scripts: Vec<PhaseScript>,
    /// The running phase's script index and how many of its steps were already sent.
    running: Option<(usize, usize)>,
    held: Vec<Key>,
}

impl<S: EventSink> ScriptedInput<S> {
    pub fn new(sink: S, scripts: Vec<PhaseScript>) -> Self {
        Self {
            sink,
            scripts,
            running: None,
            held: Vec::new(),
        }
    }

    fn send(&mut self, key: Key, down: bool) -> Result<(), &'static str> {
        self.sink.write(&encode_key(key, down))?;
        if down {
            if !self.held.contains(&key) {
                self.held.push(key);
            }
        } else {
            self.held.retain(|held| *held != key);
        }
        Ok(())
    }

    /// Release everything this input holds; never leaves a key down.
    /// A key whose release write failed stays recorded as held, so `released` says so and a later release retries it.
    fn release_all(&mut self) {
        for key in std::mem::take(&mut self.held) {
            if self.sink.write(&encode_key(key, false)).is_err() {
                self.held.push(key);
            }
        }
    }
}

impl<S: EventSink> RouteInput for ScriptedInput<S> {
    fn available(&self) -> bool {
        !self.scripts.is_empty()
    }

    fn begin_phase(&mut self, phase: &str) -> Result<(), &'static str> {
        // A phase never inherits the previous one's held keys.
        self.release_all();
        if !self.held.is_empty() {
            return Err("LongInputReleaseUnverified");
        }
        let index = self
            .scripts
            .iter()
            .position(|script| script.phase == phase)
            .ok_or("BlockedRoutePhaseHasNoScript")?;
        self.running = Some((index, 0));
        self.tick(0)
    }

    fn tick(&mut self, elapsed_ms: u64) -> Result<(), &'static str> {
        let Some((index, mut sent)) = self.running else {
            return Ok(());
        };
        while let Some(step) = self.scripts[index].steps.get(sent).copied() {
            if step.at_ms > elapsed_ms {
                break;
            }
            if elapsed_ms - step.at_ms > MAX_STEP_LATENESS_MS {
                self.release_all();
                return Err("LongInputLate");
            }
            if let Err(code) = self.send(step.key, step.down) {
                self.release_all();
                return Err(code);
            }
            sent += 1;
        }
        self.running = Some((index, sent));
        Ok(())
    }

    fn stop(&mut self) {
        self.running = None;
        self.release_all();
    }

    fn released(&self) -> bool {
        self.held.is_empty()
    }

    fn script_sha256(&self) -> String {
        let mut text = String::new();
        for script in &self.scripts {
            text.push_str(&format!("phase:{}\n", script.phase));
            for step in &script.steps {
                text.push_str(&format!(
                    "{}:{}:{}\n",
                    step.at_ms,
                    step.key.code(),
                    step.down
                ));
            }
        }
        stutter_core::hash::sha256_hex(text.as_bytes())
    }
}

#[cfg(test)]
#[path = "scripted_input_tests.rs"]
mod tests;
