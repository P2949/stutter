//! The transport half of a route-input sink. A device that injects keys must live inside the isolated seat's own
//! session (a virtual keyboard registered on the user's machine is picked up by the running compositor and delivers
//! to whatever window is focused), so this build never opens one. What it can do, and does, is carry the encoded input
//! events to a helper that runs inside that session: it connects only to a Unix socket whose path, owner and mode are
//! right and whose kernel-recorded peer is the expected user, names the display the helper's device is bound to
//! (`BoundSink`, so `IsolatedSink` refuses it unless that is the attested seat), writes whole events with a bounded
//! timeout, and reports a failed write rather than dropping it (the route then fails instead of passing as clean).

use std::{
    io::Write,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::Path,
    time::Duration,
};

use super::{EVENT_BYTES, EventSink, isolated_seat::BoundSink};

/// A write that takes longer than this is a failed delivery.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Events carried to the helper in the isolated session.
pub struct SeatSocketSink {
    stream: UnixStream,
    display: String,
}

impl SeatSocketSink {
    /// Connect to the helper's socket, refusing one that is not the expected user's private socket.
    pub fn connect(path: &Path, expected_uid: u32, display: &str) -> Result<Self, &'static str> {
        if display.is_empty() {
            return Err("SinkDisplayUnnamed");
        }
        // The path itself: a socket (not a link to one) owned by the expected user, writable by no one else.
        let meta = std::fs::symlink_metadata(path).map_err(|_| "SinkSocketMissing")?;
        if !meta.file_type().is_socket() {
            return Err("SinkSocketNotASocket");
        }
        if meta.uid() != expected_uid || meta.mode() & 0o022 != 0 {
            return Err("SinkSocketNotPrivate");
        }
        let stream = UnixStream::connect(path).map_err(|_| "SinkSocketRefused")?;
        // The kernel's record of who is at the other end must be the expected user, whatever the path says.
        let (_, peer_uid, _) =
            crate::syscall::unix_peer_credentials(&stream).map_err(|_| "SinkPeerUnknown")?;
        if peer_uid != expected_uid {
            return Err("SinkPeerNotTheExpectedUser");
        }
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(|_| "SinkSocketRefused")?;
        Ok(Self {
            stream,
            display: display.to_owned(),
        })
    }
}

impl EventSink for SeatSocketSink {
    fn write(&mut self, events: &[u8]) -> Result<(), &'static str> {
        // Whole `struct input_event` records only: a torn event is worse than none.
        if events.is_empty() || !events.len().is_multiple_of(EVENT_BYTES) {
            return Err("SinkEventsNotWhole");
        }
        self.stream
            .write_all(events)
            .and_then(|()| self.stream.flush())
            .map_err(|_| "SinkWriteFailed")
    }
}

impl BoundSink for SeatSocketSink {
    fn bound_display(&self) -> &str {
        &self.display
    }
}

/// The helper's side of the stream, so the transport and its receiver cannot drift apart: whole `struct input_event`
/// records only, each a key press or release of a key a route may use, each followed by its synchronization report.
/// Anything else (another event type, another key, a torn record, a value that is not a press or release) is refused,
/// so a helper that decodes with this can inject nothing a route could not have sent.
pub fn decode(bytes: &[u8]) -> Result<Vec<(super::Key, bool)>, &'static str> {
    use super::Key;
    if !bytes.len().is_multiple_of(2 * EVENT_BYTES) {
        return Err("SinkStreamTorn");
    }
    let field = |record: &[u8]| {
        (
            u16::from_le_bytes([record[16], record[17]]),
            u16::from_le_bytes([record[18], record[19]]),
            i32::from_le_bytes([record[20], record[21], record[22], record[23]]),
        )
    };
    bytes
        .chunks_exact(2 * EVENT_BYTES)
        .map(|pair| {
            let (key_event, sync) = pair.split_at(EVENT_BYTES);
            let (kind, code, value) = field(key_event);
            let key = Key::ALL.iter().copied().find(|key| key.code() == code);
            let (sync_kind, sync_code, sync_value) = field(sync);
            match (kind, key, value, (sync_kind, sync_code, sync_value)) {
                (1, Some(key), 0 | 1, (0, 0, 0)) => Ok((key, value == 1)),
                _ => Err("SinkEventRefused"),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "seat_socket_sink_tests.rs"]
mod tests;
