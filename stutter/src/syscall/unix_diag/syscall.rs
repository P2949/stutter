//! Bounded, read-only kernel UNIX socket diagnostics transport.
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};

const MAX_DATAGRAM: usize = 65_536;
const MAX_TOTAL: usize = 4 * 1024 * 1024;
const MAX_PACKETS: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(2);

/// Issue only SOCK_DIAG_BY_FAMILY/AF_UNIX with the peer attribute. The callback must validate
/// every netlink message and returns true only after a valid complete dump. Sender credentials
/// are checked here; this function accepts neither a caller-selected protocol nor request bytes.
pub(crate) fn unix_diag_dump(
    mut on_datagram: impl FnMut(&[u8]) -> io::Result<bool>,
) -> io::Result<()> {
    // SAFETY: socket has no borrowed pointers and returns a fresh descriptor or an error.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::NETLINK_SOCK_DIAG,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw is a newly created, uniquely owned descriptor.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: zero is valid for sockaddr_nl; the public fields below select the kernel endpoint.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    let mut request = [0u8; 40];
    request[0..4].copy_from_slice(&40u32.to_ne_bytes());
    request[4..6].copy_from_slice(&20u16.to_ne_bytes()); // SOCK_DIAG_BY_FAMILY
    request[6..8].copy_from_slice(&0x301u16.to_ne_bytes()); // REQUEST | DUMP
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    request[16] = libc::AF_UNIX as u8;
    request[20..24].copy_from_slice(&u32::MAX.to_ne_bytes()); // all states
    request[28..32].copy_from_slice(&4u32.to_ne_bytes()); // UDIAG_SHOW_PEER
    request[32..36].copy_from_slice(&u32::MAX.to_ne_bytes());
    request[36..40].copy_from_slice(&u32::MAX.to_ne_bytes());
    // SAFETY: request and kernel are live, correctly sized buffers and socket is a live netlink fd.
    let sent = unsafe {
        libc::sendto(
            socket.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            (&raw const kernel).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    if sent as usize != request.len() {
        return Err(io::Error::other("short UNIX diagnostic request"));
    }
    let deadline = Instant::now() + TIMEOUT;
    let mut total = 0usize;
    let mut bytes = vec![0u8; MAX_DATAGRAM];
    for _ in 0..MAX_PACKETS {
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "UNIX diagnostic deadline")
                })?;
            let timeout = left.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
            let mut poll = libc::pollfd {
                fd: socket.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: poll points to one initialized pollfd and remains live for the call.
            let status = unsafe { libc::poll(&raw mut poll, 1, timeout) };
            if status < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if status == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "UNIX diagnostic deadline",
                ));
            }
            if poll.revents & libc::POLLIN == 0 {
                return Err(io::Error::other("UNIX diagnostic socket failure"));
            }
            break;
        }
        // SAFETY: zero is valid initialization for a sockaddr_nl output buffer.
        let mut sender: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut sender_len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        // SAFETY: bytes and sender are writable for their stated lengths; the socket remains live.
        let count = unsafe {
            libc::recvfrom(
                socket.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                libc::MSG_TRUNC,
                (&raw mut sender).cast(),
                &raw mut sender_len,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        let count = count as usize;
        if count == 0
            || count > bytes.len()
            || sender_len as usize != std::mem::size_of::<libc::sockaddr_nl>()
            || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
            || sender.nl_pid != 0
            || sender.nl_groups != 0
        {
            return Err(io::Error::other(
                "untrusted or truncated UNIX diagnostic response",
            ));
        }
        total = total
            .checked_add(count)
            .filter(|n| *n <= MAX_TOTAL)
            .ok_or_else(|| io::Error::other("UNIX diagnostic response exceeds limit"))?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "UNIX diagnostic deadline",
            ));
        }
        let done = on_datagram(&bytes[..count])?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "UNIX diagnostic deadline",
            ));
        }
        if done {
            return Ok(());
        }
    }
    Err(io::Error::other("UNIX diagnostic packet limit exceeded"))
}
