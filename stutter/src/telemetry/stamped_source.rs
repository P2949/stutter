//! An authenticated, epoch-bound renderer-stamped frame source (opt-in prototype).
//!
//! [`StampedStream`] measures delivery latency; this module decides whether the frames may be
//! believed to come from the renderer the controller means. A producer is bound only when all of
//! these hold, each checked against the host rather than taken from the producer's word:
//!
//! * the kernel's peer credentials name the expected user and the expected process;
//! * that process is alive and its start time is the one the controller recorded (a reused pid is
//!   not the renderer);
//! * producer and controller share a `CLOCK_MONOTONIC` time namespace;
//! * its hello announces a known protocol version, the `CLOCK_MONOTONIC` clock, a supported frame
//!   event (`present`), a source epoch greater than every epoch already used (a replayed hello is
//!   refused) and the epoch's first sequence number.
//!
//! One producer at a time: while a bound producer is alive a second claim is refused as competing.
//! A producer that exits or is replaced withdraws the source until a new, higher epoch binds, and
//! every bind resets the stream's trust.
//!
//! Delivery latency is not stamp accuracy. The alignment uncertainty reported to the gate is the
//! measured delivery bound **plus** a stamp-error bound that must be supplied from an independent
//! reference ([`StampAccuracy`]); without one the uncertainty is unknown (`NaN`) and the gate
//! denies, however fast the socket is. A `StampAccuracy` cannot be written as a literal: it is built
//! only from the raw pairs of (renderer stamp, independent reference time) that were measured, the
//! bound is recomputed from them, and it is accepted only for the renderer binary, frame event and boot
//! it was measured for, within its validity window. This module cannot produce that reference: it needs a real
//! instrumented renderer measured against something outside the renderer.
use std::{fs, path::PathBuf};

use serde::Deserialize;

use super::{
    frame_source::Attribution,
    proc_view::{ClockNamespaceDenial, live_process, shared_monotonic_namespace},
    quality::StreamQuality,
    registration::PeerCredentials,
    stamped_frames::{StampError, StampedFrame, StampedStream},
};

pub const PROTOCOL_VERSION: u32 = 1;
/// The frame events a producer may stamp. Only the present call has defined semantics here.
pub const SUPPORTED_EVENTS: [&str; 1] = ["present"];
/// The longest frame line accepted; a real frame record is under 100 bytes.
pub const MAX_STAMPED_LINE_BYTES: usize = 256;
/// Fewest reference samples a stamp-accuracy measurement may rest on.
pub const MIN_ACCURACY_SAMPLES: usize = 1000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub version: u32,
    pub epoch: u64,
    pub first_seq: u64,
    /// The pipeline event each stamp is sampled at.
    pub event: String,
    pub clock: String,
}

/// Who the controller expects the producer to be.
#[derive(Clone, Debug)]
pub struct SourcePolicy {
    pub owner_uid: u32,
    pub renderer_pid: u32,
    /// The renderer's start time when the controller recorded it.
    pub renderer_start_ticks: u64,
    /// The controller's own pid, for the shared time namespace.
    pub observer_pid: u32,
    pub proc_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindDenial {
    WrongUser,
    WrongProcess,
    /// The process is gone, or its start time is not the recorded one (a reused pid).
    IdentityChanged,
    Namespace(ClockNamespaceDenial),
    BadHello,
    UnsupportedEvent,
    /// The epoch is not greater than one already used.
    ReplayedEpoch,
    /// A bound producer is still alive.
    CompetingProducer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestError {
    NotBound,
    Malformed,
    /// The line is longer than [`MAX_STAMPED_LINE_BYTES`].
    Oversize,
    Rejected(StampError),
}

/// What the independent reference was. Only references outside the renderer count: the renderer
/// cannot grade its own timestamps, and there is deliberately no free-form or "self" kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Photodiode,
    DisplayHardwareTimestamp,
    HighSpeedCamera,
}

/// Why a stamp-accuracy claim was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccuracyRefusal {
    /// Fewer than [`MIN_ACCURACY_SAMPLES`] pairs.
    TooFewSamples,
    /// More pairs than any real measurement needs.
    TooManySamples,
    /// A line is not two unsigned integers, or a stamp or reference is out of range.
    Malformed,
    /// The validity window has already ended.
    Expired,
    /// No renderer is bound, so there is nothing the claim could describe.
    NotBound,
    /// The claim was measured for another renderer binary than the one bound.
    WrongRenderer,
    /// The claim was measured for another frame event than the one the producer announced.
    WrongEvent,
    /// The claim was measured in another boot.
    WrongBoot,
}

/// A measured bound on how far a stamp can differ from the real frame event, computed from raw
/// (stamp, reference) pairs. It rests on at least [`MIN_ACCURACY_SAMPLES`] of them and is bound to the
/// renderer binary, the frame event and the boot it was measured for, within a validity window. Its
/// fields are private so it cannot be built any other way.
///
/// ```compile_fail
/// use stutter::api::telemetry::{ReferenceKind, StampAccuracy};
/// let forged = StampAccuracy {
///     max_error_ns: 0,
///     bias_ns: 0,
///     samples: 1000,
///     reference: ReferenceKind::Photodiode,
///     renderer_exe_sha256: String::new(),
///     event: String::new(),
///     boot_id: String::new(),
///     valid_until_unix: 0,
///     evidence_sha256: String::new(),
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampAccuracy {
    max_error_ns: u64,
    bias_ns: i64,
    samples: usize,
    reference: ReferenceKind,
    renderer_exe_sha256: String,
    event: String,
    boot_id: String,
    valid_until_unix: u64,
    evidence_sha256: String,
}

/// The most pairs one measurement may hold.
pub const MAX_ACCURACY_SAMPLES: usize = 1_000_000;

impl StampAccuracy {
    /// Builds a claim from raw measurement text: one `stamp_ns reference_ns` pair per line, both on
    /// the shared clock. The bound is the largest absolute difference (the bias is kept for the
    /// record, not subtracted: an unmodelled constant offset is still error). Nothing about the
    /// claim is taken on trust except that the pairs were measured.
    pub fn from_raw_pairs(
        reference: ReferenceKind,
        renderer_exe_sha256: &str,
        event: &str,
        boot_id: &str,
        valid_until_unix: u64,
        now_unix: u64,
        raw: &str,
    ) -> Result<Self, AccuracyRefusal> {
        if valid_until_unix <= now_unix {
            return Err(AccuracyRefusal::Expired);
        }
        let mut samples = 0_usize;
        let (mut max_error, mut sum) = (0_u64, 0_i128);
        for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let mut fields = line.split_whitespace();
            let (Some(stamp), Some(truth), None) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(AccuracyRefusal::Malformed);
            };
            let (Ok(stamp), Ok(truth)) = (stamp.parse::<u64>(), truth.parse::<u64>()) else {
                return Err(AccuracyRefusal::Malformed);
            };
            let error = i128::from(stamp) - i128::from(truth);
            max_error = max_error.max(u64::try_from(error.unsigned_abs()).unwrap_or(u64::MAX));
            sum += error;
            samples += 1;
            if samples > MAX_ACCURACY_SAMPLES {
                return Err(AccuracyRefusal::TooManySamples);
            }
        }
        if samples < MIN_ACCURACY_SAMPLES {
            return Err(AccuracyRefusal::TooFewSamples);
        }
        let bias = (sum / samples as i128).clamp(i128::from(i64::MIN), i128::from(i64::MAX));
        Ok(Self {
            max_error_ns: max_error,
            bias_ns: bias as i64,
            samples,
            reference,
            renderer_exe_sha256: renderer_exe_sha256.to_ascii_lowercase(),
            event: event.to_owned(),
            boot_id: boot_id.trim().to_owned(),
            valid_until_unix,
            evidence_sha256: stutter_core::hash::sha256_hex(raw.as_bytes()),
        })
    }

    pub fn max_error_ns(&self) -> u64 {
        self.max_error_ns
    }

    pub fn bias_ns(&self) -> i64 {
        self.bias_ns
    }

    pub fn samples(&self) -> usize {
        self.samples
    }

    pub fn reference(&self) -> ReferenceKind {
        self.reference
    }

    /// The digest of the raw pairs the claim was computed from, for the evidence record.
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }
}

#[derive(Debug)]
struct Bound {
    pid: u32,
    start_ticks: u64,
    /// The frame event the producer announced in its hello.
    event: String,
}

#[derive(Debug)]
pub struct StampedSource {
    policy: SourcePolicy,
    stream: StampedStream,
    bound: Option<Bound>,
    last_epoch: Option<u64>,
    accuracy: Option<StampAccuracy>,
}

impl StampedSource {
    pub fn new(policy: SourcePolicy) -> Self {
        Self {
            policy,
            stream: StampedStream::default(),
            bound: None,
            last_epoch: None,
            accuracy: None,
        }
    }

    pub fn stream(&self) -> &StampedStream {
        &self.stream
    }

    pub fn is_bound(&self) -> bool {
        self.bound.is_some()
    }

    /// Records a stamp-accuracy claim for the bound renderer. It is accepted only if a renderer is
    /// bound, the claim was measured for *that* binary (its sha256 is recomputed here from the
    /// process's executable), for the frame event the producer announced, in this boot, and its
    /// validity window has not ended. It describes this renderer, so it is dropped whenever a
    /// different process binds, and expiry is checked again whenever quality is decided.
    pub fn set_stamp_accuracy(
        &mut self,
        accuracy: StampAccuracy,
        now_unix: u64,
    ) -> Result<(), AccuracyRefusal> {
        self.revalidate();
        let bound = self.bound.as_ref().ok_or(AccuracyRefusal::NotBound)?;
        if accuracy.valid_until_unix <= now_unix {
            return Err(AccuracyRefusal::Expired);
        }
        if accuracy.event != bound.event {
            return Err(AccuracyRefusal::WrongEvent);
        }
        let boot = fs::read_to_string(self.policy.proc_root.join("sys/kernel/random/boot_id"))
            .unwrap_or_default();
        if boot.trim().is_empty() || boot.trim() != accuracy.boot_id {
            return Err(AccuracyRefusal::WrongBoot);
        }
        let exe = fs::read(self.policy.proc_root.join(format!("{}/exe", bound.pid)))
            .map_err(|_| AccuracyRefusal::WrongRenderer)?;
        if stutter_core::hash::sha256_hex(&exe) != accuracy.renderer_exe_sha256 {
            return Err(AccuracyRefusal::WrongRenderer);
        }
        self.accuracy = Some(accuracy);
        Ok(())
    }

    /// The producer is bound only while it is still the recorded process.
    pub fn revalidate(&mut self) {
        let still = self.bound.as_ref().is_some_and(|bound| {
            live_process(&self.policy.proc_root, bound.pid)
                .is_some_and(|live| live.start_ticks == bound.start_ticks)
        });
        if !still {
            self.bound = None;
        }
    }

    /// Binds the producer behind `peer` after its `hello` line. See the module documentation for
    /// every check. A refusal changes nothing.
    pub fn bind(&mut self, peer: PeerCredentials, hello_line: &str) -> Result<(), BindDenial> {
        self.revalidate();
        if self.bound.is_some() {
            return Err(BindDenial::CompetingProducer);
        }
        if peer.uid != self.policy.owner_uid {
            return Err(BindDenial::WrongUser);
        }
        if peer.pid != self.policy.renderer_pid {
            return Err(BindDenial::WrongProcess);
        }
        let live =
            live_process(&self.policy.proc_root, peer.pid).ok_or(BindDenial::IdentityChanged)?;
        if live.start_ticks != self.policy.renderer_start_ticks {
            return Err(BindDenial::IdentityChanged);
        }
        shared_monotonic_namespace(&self.policy.proc_root, peer.pid, self.policy.observer_pid)
            .map_err(BindDenial::Namespace)?;
        let hello: Hello =
            serde_json::from_str(hello_line.trim()).map_err(|_| BindDenial::BadHello)?;
        if hello.version != PROTOCOL_VERSION || hello.clock != "CLOCK_MONOTONIC" {
            return Err(BindDenial::BadHello);
        }
        if !SUPPORTED_EVENTS.contains(&hello.event.as_str()) {
            return Err(BindDenial::UnsupportedEvent);
        }
        if self.last_epoch.is_some_and(|last| hello.epoch <= last) {
            return Err(BindDenial::ReplayedEpoch);
        }
        if self
            .bound
            .as_ref()
            .is_none_or(|bound| bound.pid != peer.pid || bound.start_ticks != live.start_ticks)
        {
            // A different renderer than the one any earlier measurement described.
            self.accuracy = None;
        }
        self.last_epoch = Some(hello.epoch);
        self.stream.begin_epoch(hello.epoch, hello.first_seq);
        self.bound = Some(Bound {
            pid: peer.pid,
            start_ticks: live.start_ticks,
            event: hello.event.clone(),
        });
        Ok(())
    }

    /// Reads the peer credentials from the kernel for a connected socket, then binds.
    pub fn accept(
        &mut self,
        socket: &std::os::unix::net::UnixStream,
        hello_line: &str,
    ) -> Result<(), BindDenial> {
        let peer = super::registration::kernel_peer_credentials(socket)
            .map_err(|_| BindDenial::WrongProcess)?;
        self.bind(peer, hello_line)
    }

    /// Validates one frame line against the controller's reading of the shared clock.
    pub fn ingest(&mut self, line: &str, rx_ns: u64) -> Result<u64, IngestError> {
        self.revalidate();
        if self.bound.is_none() {
            return Err(IngestError::NotBound);
        }
        // A message that never becomes a frame is loss: a malformed, truncated or oversize line
        // leaves the stream's boundary unproven, so it is counted and the stream's quality says so.
        if line.len() > MAX_STAMPED_LINE_BYTES {
            self.stream.record_invalid();
            return Err(IngestError::Oversize);
        }
        let Ok(frame) = serde_json::from_str::<StampedFrame>(line.trim()) else {
            self.stream.record_invalid();
            return Err(IngestError::Malformed);
        };
        self.stream
            .push(frame, rx_ns)
            .map_err(IngestError::Rejected)
    }

    /// The quality for the frame-primary gate, decided now: the renderer's identity is rechecked
    /// first, so a renderer that exited (or whose pid was reused) since the last frame is no longer
    /// `ValidatedDirect`. The alignment is the delivery bound plus the stamp-error bound, and is
    /// unknown until both exist; an expired accuracy claim is as good as none.
    pub fn quality_at(&mut self, now_ns: u64) -> StreamQuality {
        self.revalidate();
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(u64::MAX, |elapsed| elapsed.as_secs());
        if self
            .accuracy
            .as_ref()
            .is_some_and(|accuracy| accuracy.valid_until_unix <= now_unix)
        {
            self.accuracy = None;
        }
        let bound = self.bound.is_some();
        let mut quality = self.stream.quality_at(
            now_ns,
            if bound {
                Attribution::ValidatedDirect
            } else {
                Attribution::Unavailable
            },
            bound,
        );
        quality.alignment_uncertainty_ms = match (&self.accuracy, self.stream.calibration()) {
            (Some(accuracy), Some(delivery)) => {
                delivery.uncertainty_ms() + accuracy.max_error_ns as f64 / 1_000_000.0
            }
            _ => f64::NAN,
        };
        quality
    }
}

#[cfg(test)]
#[path = "stamped_source_tests.rs"]
mod tests;
