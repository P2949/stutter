//! The one place the userspace loader registers a kernel-owned descriptor with the async runtime.
use aya::maps::{MapData, RingBuf};
use tokio::io::unix::AsyncFd;

/// Registers the events ring buffer for readiness notifications.
pub(crate) fn register_ring_buffer(
    events: RingBuf<MapData>,
) -> std::io::Result<AsyncFd<RingBuf<MapData>>> {
    // SAFETY: `events` is an aya `RingBuf` that owns the map file descriptor for its whole
    // lifetime, and the returned `AsyncFd` takes ownership of the `RingBuf`: the descriptor
    // stays valid and open for exactly as long as the registration, and nothing else closes it.
    unsafe { AsyncFd::register(events) }.map_err(|error| error.into_parts().1)
}
