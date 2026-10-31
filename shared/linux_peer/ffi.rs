//! Kernel peer identity shared by the product worker and validation runner.
use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
};

/// Peer identity of a connected Unix socket, as reported by the kernel (`SO_PEERCRED`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

pub fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` and `length` are valid for writes of the sizes passed, the fd is
    // owned by `stream` for the duration of the call, and SO_PEERCRED only fills a `ucred`.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() || credentials.pid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid kernel peer credentials",
        ));
    }
    Ok(PeerCredentials {
        pid: credentials.pid as u32,
        uid: credentials.uid,
        gid: credentials.gid,
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;
    #[test]
    fn credentials_come_from_the_connected_kernel_socket() {
        let (client, server) = UnixStream::pair().unwrap();
        let expected_uid = std::fs::metadata("/proc/self").unwrap().uid();
        for socket in [&client, &server] {
            let peer = peer_credentials(socket).unwrap();
            assert_eq!(peer.pid, std::process::id());
            assert_eq!(peer.uid, expected_uid);
        }
    }
}
