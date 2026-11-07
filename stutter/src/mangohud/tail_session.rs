//! Bounded, polled collection of a MangoHud CSV log, independent of any recording.
//!
//! The session opens the log relative to a verified capture directory, so a symlink, traversal or
//! special file is refused. It is a synchronous state machine: the caller polls it and passes the
//! room it has for frames, so nothing is dropped silently, because unread bytes simply stay in the
//! file until there is room. A truncated, replaced or rotated source starts a new epoch with a
//! fresh header and parser, so frames from different epochs are never mixed.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

use super::{
    parser::MangoHudLiveParser,
    schema::{schema_from_headers, split_csv_line},
};
use crate::{
    recorder::FrameEvent,
    telemetry::session::{ConfinedDir, ConfinementError, FileIdentity},
};

#[derive(Clone, Copy, Debug)]
pub struct TailConfig {
    /// How long the log may take to appear before the session fails.
    pub create_deadline: Duration,
    /// Longest accepted line; a longer one is discarded to its newline and counted.
    pub max_line_bytes: usize,
    /// Most bytes read from the file by one poll.
    pub max_read_bytes: usize,
    /// Non-matching lines tolerated before a recognized header must have appeared.
    pub max_header_lines: usize,
}

impl Default for TailConfig {
    fn default() -> Self {
        Self {
            create_deadline: Duration::from_secs(30),
            max_line_bytes: 4096,
            max_read_bytes: 64 * 1024,
            max_header_lines: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TailCounters {
    pub frames: u64,
    pub oversize_lines_discarded: u64,
    pub truncations: u64,
    pub replacements: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailFailure {
    /// The log did not appear before the deadline.
    NeverCreated,
    /// No recognized frame header within the allowed number of lines.
    NoHeader,
    /// The log name resolved to something unsafe (symlink, special file, wrong owner).
    UnsafeSource,
    Io,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailState {
    WaitingForFile,
    WaitingForHeader,
    Streaming,
    Failed(TailFailure),
}

/// A frame with the source epoch it belongs to.
#[derive(Clone, Debug)]
pub struct TailedFrame {
    pub epoch: u64,
    pub event: FrameEvent,
}

struct Open {
    file: File,
    identity: FileIdentity,
    offset: u64,
}

pub struct MangoHudTailSession {
    dir: ConfinedDir,
    name: String,
    config: TailConfig,
    started: Instant,
    now: Instant,
    open: Option<Open>,
    parser: Option<MangoHudLiveParser>,
    pending: Vec<u8>,
    discarding_oversize: bool,
    header_lines_seen: usize,
    epoch: u64,
    counters: TailCounters,
    state: TailState,
}

impl MangoHudTailSession {
    pub fn new(dir: ConfinedDir, name: &str, config: TailConfig, now: Instant) -> Self {
        Self {
            dir,
            name: name.to_owned(),
            config,
            started: now,
            now,
            open: None,
            parser: None,
            pending: Vec::new(),
            discarding_oversize: false,
            header_lines_seen: 0,
            epoch: 0,
            counters: TailCounters::default(),
            state: TailState::WaitingForFile,
        }
    }

    pub fn state(&self) -> TailState {
        self.state
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn counters(&self) -> TailCounters {
        self.counters
    }

    pub fn identity(&self) -> Option<FileIdentity> {
        self.open.as_ref().map(|open| open.identity)
    }

    fn reset_stream(&mut self) {
        self.parser = None;
        self.pending.clear();
        self.discarding_oversize = false;
        self.header_lines_seen = 0;
        self.state = TailState::WaitingForHeader;
    }

    /// Notices a replaced file (a different device/inode under the same name) or a truncation.
    fn detect_source_change(&mut self) -> Result<(), TailFailure> {
        match self.dir.open_file(&self.name) {
            Ok((file, identity)) => {
                let changed = self
                    .open
                    .as_ref()
                    .is_some_and(|open| open.identity != identity);
                if self.open.is_none() || changed {
                    if changed {
                        self.counters.replacements += 1;
                        self.epoch += 1;
                    }
                    self.open = Some(Open {
                        file,
                        identity,
                        offset: 0,
                    });
                    self.reset_stream();
                }
            }
            Err(ConfinementError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                // Not created yet, or removed while still open: keep reading what is open.
                if self.open.is_none() && self.started_elapsed() > self.config.create_deadline {
                    return Err(TailFailure::NeverCreated);
                }
            }
            Err(ConfinementError::Io(error)) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(TailFailure::UnsafeSource);
            }
            Err(ConfinementError::Io(_)) => return Err(TailFailure::Io),
            Err(_) => return Err(TailFailure::UnsafeSource),
        }
        let truncated = match self.open.as_ref() {
            Some(open) => open.file.metadata().map_err(|_| TailFailure::Io)?.len() < open.offset,
            None => false,
        };
        if truncated {
            self.counters.truncations += 1;
            self.epoch += 1;
            if let Some(open) = self.open.as_mut() {
                open.file
                    .seek(SeekFrom::Start(0))
                    .map_err(|_| TailFailure::Io)?;
                open.offset = 0;
            }
            self.reset_stream();
        }
        Ok(())
    }

    fn started_elapsed(&self) -> Duration {
        self.now.saturating_duration_since(self.started)
    }

    /// One poll: appends at most `room` frames to `out` and returns the resulting state. With no
    /// room nothing is read, so no frame is lost to a full downstream.
    pub fn poll(&mut self, now: Instant, room: usize, out: &mut Vec<TailedFrame>) -> TailState {
        if matches!(self.state, TailState::Failed(_)) {
            return self.state;
        }
        self.now = now;
        if let Err(failure) = self.detect_source_change() {
            self.state = TailState::Failed(failure);
            return self.state;
        }
        let mut produced = 0;
        loop {
            if let Err(failure) = self.consume_lines(room, &mut produced, out) {
                self.state = TailState::Failed(failure);
                return self.state;
            }
            if produced >= room {
                break;
            }
            let Some(open) = self.open.as_mut() else {
                break;
            };
            let mut chunk = vec![0u8; self.config.max_read_bytes.min(8192)];
            match open.file.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    open.offset += read as u64;
                    self.pending.extend_from_slice(&chunk[..read]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.state = TailState::Failed(TailFailure::Io);
                    return self.state;
                }
            }
        }
        self.state
    }

    /// Parses complete lines from `pending` until `room` frames were produced. Whatever is left
    /// stays for the next poll; an over-long partial line is discarded to its newline.
    fn consume_lines(
        &mut self,
        room: usize,
        produced: &mut usize,
        out: &mut Vec<TailedFrame>,
    ) -> Result<(), TailFailure> {
        while *produced < room {
            let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') else {
                if self.pending.len() > self.config.max_line_bytes {
                    if !self.discarding_oversize {
                        self.counters.oversize_lines_discarded += 1;
                    }
                    self.discarding_oversize = true;
                    self.pending.clear();
                }
                return Ok(());
            };
            let raw: Vec<u8> = self.pending.drain(..=newline).collect();
            if self.discarding_oversize {
                self.discarding_oversize = false;
                continue;
            }
            if raw.len() > self.config.max_line_bytes {
                self.counters.oversize_lines_discarded += 1;
                continue;
            }
            let text = String::from_utf8_lossy(&raw);
            let line = text.trim_end_matches(['\n', '\r']);
            match self.parser.as_mut() {
                None => {
                    self.header_lines_seen += 1;
                    if let Some(schema) = schema_from_headers(&split_csv_line(line)) {
                        self.parser = Some(MangoHudLiveParser::new(schema));
                        self.state = TailState::Streaming;
                    } else if self.header_lines_seen > self.config.max_header_lines {
                        return Err(TailFailure::NoHeader);
                    }
                }
                Some(parser) => {
                    if let Some(event) = parser.parse_line(line) {
                        self.counters.frames += 1;
                        *produced += 1;
                        out.push(TailedFrame {
                            epoch: self.epoch,
                            event,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tail_session_tests.rs"]
mod tests;

/// Drives `session` against a shared status and a bounded frame channel until the channel closes
/// or the session fails. Frames the channel cannot take are left in the file, never dropped; a
/// failed session marks the source unhealthy so frame-primary trust ends immediately.
pub async fn drive_tail_session(
    mut session: MangoHudTailSession,
    status: crate::telemetry::live_source::SharedFrameSource,
    tx: tokio::sync::mpsc::Sender<TailedFrame>,
    idle_sleep: Duration,
) {
    let mut frames = Vec::new();
    loop {
        frames.clear();
        let room = tx.capacity();
        let state = session.poll(Instant::now(), room, &mut frames);
        let produced = frames.len();
        let mut closed = false;
        for frame in frames.drain(..) {
            if tx.try_send(frame).is_err() {
                // Capacity was just checked, so only a closed receiver lands here.
                closed = true;
                break;
            }
        }
        if let Ok(mut shared) = status.lock() {
            if produced > 0 {
                shared.last_frame_at = Some(Instant::now());
            }
            shared.healthy = !matches!(state, TailState::Failed(_));
        }
        if closed || matches!(state, TailState::Failed(_)) || tx.is_closed() {
            return;
        }
        tokio::time::sleep(idle_sleep).await;
    }
}
