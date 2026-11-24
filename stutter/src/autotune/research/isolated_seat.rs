//! The only way a device sink may ever reach the game. A virtual keyboard registered on the user's machine is
//! picked up by the running compositor and delivers its keys to whatever window is focused, so a route's inputs may
//! be written only to a seat of their own. This module makes that a precondition the type system carries:
//!
//! * `attest` turns the game process's own environment (read from the kernel) and the user's live session into an
//!   `IsolatedSeat`, and refuses unless the game runs on a display of its own: a Wayland display and an X display
//!   that are not the live session's, no Hyprland instance, not the live session id, and a logind seat other than
//!   `seat0`. An absent or shared display or seat is refused, never assumed isolated.
//! * `IsolatedSink` wraps a device sink and can only be built from an attested seat and a sink that names the very
//!   display it is bound to; a sink bound anywhere else (above all, the live display) is refused at construction and
//!   can never be written through.
//!
//! No physical sink exists in this repository, and none is supplied here: the build keeps `UnavailableInput`. This
//! is the gate a future sink must pass, exercised with fixtures that touch no device.

use std::collections::BTreeMap;

use super::EventSink;

/// The user's live interactive session, whose displays and id an isolated seat must not share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    pub wayland_display: String,
    pub x_display: String,
    pub session_id: String,
}

/// A seat of its own: the display the game runs on, proven not to be the live session's. It cannot be built except by
/// `attest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsolatedSeat {
    display: String,
    seat: String,
}

impl IsolatedSeat {
    /// The Wayland display the game runs on.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// The logind seat of the game's own session (`XDG_SEAT` of its environment), never `seat0`. A device is on the
    /// isolated seat only when udev attaches it to this one: the name is the game's, not a caller's.
    pub fn seat(&self) -> &str {
        &self.seat
    }
}

fn environment(environ: &[u8]) -> BTreeMap<String, String> {
    environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let text = String::from_utf8_lossy(entry);
            let (key, value) = text.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Whether the game process (its `/proc/<pid>/environ`) runs on a seat of its own, apart from `live`.
pub fn attest(game_environ: &[u8], live: &LiveSession) -> Result<IsolatedSeat, &'static str> {
    let env = environment(game_environ);
    let wayland = env
        .get("WAYLAND_DISPLAY")
        .filter(|value| !value.is_empty())
        .ok_or("GameDisplayUnknown")?;
    let x = env
        .get("DISPLAY")
        .filter(|value| !value.is_empty())
        .ok_or("GameDisplayUnknown")?;
    if live.wayland_display.is_empty() || live.x_display.is_empty() || live.session_id.is_empty() {
        // Without the live session's own identifiers nothing can be shown different from them.
        return Err("LiveSessionUnknown");
    }
    if env.contains_key("HYPRLAND_INSTANCE_SIGNATURE") {
        return Err("GameOnTheLiveCompositor");
    }
    if *wayland == live.wayland_display || *x == live.x_display {
        return Err("GameSharesTheLiveDisplay");
    }
    if env
        .get("XDG_SESSION_ID")
        .is_none_or(|id| *id == live.session_id)
    {
        return Err("GameSharesTheLiveSession");
    }
    // The seat logind gave the game's session: absent is unknown, and seat0 is the live session's own seat.
    let seat = env
        .get("XDG_SEAT")
        .filter(|value| !value.is_empty())
        .ok_or("GameSeatUnknown")?;
    if seat == "seat0" {
        return Err("GameOnTheLiveSeat");
    }
    Ok(IsolatedSeat {
        display: wayland.clone(),
        seat: seat.clone(),
    })
}

/// Where logind keeps one record per session (`<id>`, with `SEAT=` and `UID=` lines).
pub const LOGIND_SESSIONS: &str = "/run/systemd/sessions";

/// As `attest`, and the seat the game's environment names must also be the one logind itself recorded for that session
/// and for this user: an environment is the launcher's claim, the session record is the system's.
pub fn attest_confirmed(
    game_environ: &[u8],
    live: &LiveSession,
    logind_dir: &std::path::Path,
    uid: u32,
) -> Result<IsolatedSeat, &'static str> {
    let seat = attest(game_environ, live)?;
    let id = environment(game_environ)
        .get("XDG_SESSION_ID")
        .cloned()
        .unwrap_or_default();
    // The id names a file: nothing but a session id's own characters may reach the path.
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("GameSessionNotKnownToLogind");
    }
    let record =
        std::fs::read_to_string(logind_dir.join(&id)).map_err(|_| "GameSessionNotKnownToLogind")?;
    let field = |name: &str| {
        record.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
        })
    };
    if field("SEAT") != Some(seat.seat()) || field("UID") != Some(uid.to_string().as_str()) {
        return Err("GameSeatNotConfirmedByLogind");
    }
    Ok(seat)
}

/// A device sink that says which display its device is bound to.
pub trait BoundSink: EventSink {
    fn bound_display(&self) -> &str;
}

/// A sink that may be written through only because it is bound to the attested isolated seat.
pub struct IsolatedSink<S: BoundSink> {
    inner: S,
}

impl<S: BoundSink> IsolatedSink<S> {
    pub fn new(inner: S, seat: &IsolatedSeat) -> Result<Self, &'static str> {
        if inner.bound_display() != seat.display() {
            return Err("SinkNotOnTheIsolatedSeat");
        }
        Ok(Self { inner })
    }
}

impl<S: BoundSink> EventSink for IsolatedSink<S> {
    fn write(&mut self, events: &[u8]) -> Result<(), &'static str> {
        self.inner.write(events)
    }
}

#[cfg(test)]
#[path = "isolated_seat_tests.rs"]
mod tests;
