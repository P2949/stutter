//! The local-only socket front end of the KPI store.
//!
//! One framed schema: newline-delimited JSON samples, one reply line per sample. The peer's
//! credentials come from the kernel (`SO_PEERCRED`) and its start time is read live from procfs, so
//! neither can be claimed by the sender. Nothing on this socket executes anything or mutates the
//! system; a reply only says whether the sample entered the store.
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use super::ingest::{IngestDenial, MAX_SAMPLE_LINE_BYTES, SampleStore};
use crate::telemetry::{proc_view::live_process, registration::PeerCredentials};

/// Most samples accepted from one connection.
pub const MAX_SAMPLES_PER_CONNECTION: usize = 1024;

/// The stable code a denial is reported with.
pub fn denial_code(denial: IngestDenial) -> &'static str {
    match denial {
        IngestDenial::LineTooLong => "line_too_long",
        IngestDenial::BadJson => "bad_json",
        IngestDenial::WrongUser => "wrong_user",
        IngestDenial::NotTheBoundProcess => "not_the_bound_process",
        IngestDenial::PeerStartTimeMismatch => "peer_start_time_mismatch",
        IngestDenial::UnknownMetric => "unknown_metric",
        IngestDenial::NonFinite => "non_finite",
        IngestDenial::NegativeValue => "negative_value",
        IngestDenial::NotAfterPrevious => "not_after_previous",
        IngestDenial::FutureTimestamp => "future_timestamp",
        IngestDenial::TooOld => "too_old",
        IngestDenial::EmptyPopulation => "empty_population",
        IngestDenial::RawEventPopulation => "raw_event_population",
    }
}

/// Serves one connection: authenticates the peer once, then answers each sample line. Returns how
/// many samples were accepted. `now_ns` is the local monotonic clock, read per sample.
pub fn serve_connection(
    store: &mut SampleStore,
    stream: UnixStream,
    proc_root: &Path,
    mut now_ns: impl FnMut() -> u64,
) -> std::io::Result<usize> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let (pid, uid, _) = crate::syscall::unix_peer_credentials(&stream)?;
    let peer = PeerCredentials { pid, uid };
    // A process that already exited has no start time and can match no binding.
    let start_ticks = live_process(proc_root, pid).map(|live| live.start_ticks);
    let budget = (MAX_SAMPLES_PER_CONNECTION * (MAX_SAMPLE_LINE_BYTES + 1)) as u64;
    let mut reader = BufReader::new((&stream).take(budget));
    let mut writer = &stream;
    let mut accepted = 0;
    let mut line = Vec::new();
    for _ in 0..MAX_SAMPLES_PER_CONNECTION {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let reply = match (std::str::from_utf8(&line), start_ticks) {
            (_, None) => Err(IngestDenial::PeerStartTimeMismatch),
            (Err(_), _) => Err(IngestDenial::BadJson),
            (Ok(text), Some(ticks)) => {
                store.ingest_line(peer, ticks, text.trim_end_matches(['\n', '\r']), now_ns())
            }
        };
        let answer = match reply {
            Ok(()) => {
                accepted += 1;
                "ok\n".to_owned()
            }
            Err(denial) => format!("denied {}\n", denial_code(denial)),
        };
        writer.write_all(answer.as_bytes())?;
    }
    Ok(accepted)
}
