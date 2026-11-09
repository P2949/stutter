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
//! denies, however fast the socket is. This module cannot produce that reference: it needs a real
//! instrumented renderer measured against something outside the renderer.
use std::path::PathBuf;

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
    Rejected(StampError),
}

/// A measured bound on how far a stamp can differ from the real frame event, from an independent
/// reference. It rests on at least [`MIN_ACCURACY_SAMPLES`] comparisons and names its reference.
#[derive(Clone, Debug, PartialEq)]
pub struct StampAccuracy {
    pub max_error_ns: u64,
    pub samples: usize,
    pub reference: String,
}

#[derive(Debug)]
struct Bound {
    pid: u32,
    start_ticks: u64,
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

    /// Records an independent measurement of stamp accuracy. A measurement with too few samples
    /// or no named reference is refused. It describes this renderer, so it is dropped whenever a
    /// different process binds.
    pub fn set_stamp_accuracy(&mut self, accuracy: StampAccuracy) -> bool {
        let valid =
            accuracy.samples >= MIN_ACCURACY_SAMPLES && !accuracy.reference.trim().is_empty();
        if valid {
            self.accuracy = Some(accuracy);
        }
        valid
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
        let frame: StampedFrame =
            serde_json::from_str(line.trim()).map_err(|_| IngestError::Malformed)?;
        self.stream
            .push(frame, rx_ns)
            .map_err(IngestError::Rejected)
    }

    /// The quality for the frame-primary gate. The attribution is `ValidatedDirect` only while a
    /// producer is bound; the alignment is the delivery bound plus the stamp-error bound, and is
    /// unknown until both exist.
    pub fn quality_at(&self, now_ns: u64) -> StreamQuality {
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
