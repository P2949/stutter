//! Best-effort console echo must not block bounded capture on a full pipe or socket.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileTypeExt, OpenOptionsExt},
    },
};

pub(super) enum ConsoleSink {
    File(File),
    Socket(File),
}

impl ConsoleSink {
    pub(super) fn open(stderr: bool) -> Option<Self> {
        Self::from_fd(if stderr { 2 } else { 1 })
    }

    fn from_fd(fd: i32) -> Option<Self> {
        // SAFETY: F_DUPFD_CLOEXEC duplicates the supplied descriptor into a fresh owned fd.
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        if copy < 0 {
            return None;
        }
        // SAFETY: successful fcntl returned a fresh descriptor owned by this scope.
        let file = unsafe { File::from_raw_fd(copy) };
        let kind = file.metadata().ok()?.file_type();
        if kind.is_socket() {
            return Some(Self::Socket(file));
        }
        if kind.is_file() {
            // Preserve the original shared file offset/append semantics for regular logs.
            return Some(Self::File(file));
        }
        if kind.is_fifo() || kind.is_char_device() {
            // Reopening procfd creates an independent open description: O_NONBLOCK must not
            // change the original stdout/stderr flags used by the rest of the application.
            let sink = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(format!("/proc/self/fd/{}", file.as_raw_fd()))
                .ok()?;
            return Some(Self::File(sink));
        }
        None
    }

    pub(super) fn echo(&mut self, bytes: &[u8]) -> usize {
        match self {
            Self::File(file) => file.write(bytes).unwrap_or(0),
            Self::Socket(file) => {
                // SAFETY: file owns a socket fd and bytes is valid for the supplied length.
                let written = unsafe {
                    libc::send(
                        file.as_raw_fd(),
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                usize::try_from(written).unwrap_or(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regular_file_echo_preserves_the_original_write_offset() {
        let path = std::env::temp_dir().join(format!("stutter-tee-offset-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(b"before\n").unwrap();
        let mut sink = ConsoleSink::from_fd(file.as_raw_fd()).unwrap();
        assert_eq!(sink.echo(b"after\n"), 6);
        assert_eq!(std::fs::read(&path).unwrap(), b"before\nafter\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn socket_backpressure_is_per_call_without_changing_original_flags() {
        let (_reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        // SAFETY: writer owns a valid descriptor; F_GETFL only reads its flags.
        let before = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
        let mut sink = ConsoleSink::from_fd(writer.as_raw_fd()).unwrap();
        let dropped = (0..1000).filter(|_| sink.echo(&[b'x'; 8192]) == 0).count();
        assert!(dropped > 0);
        // SAFETY: writer still owns the same valid descriptor.
        let after = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(after, before);
    }
    #[test]
    fn reopening_a_pipe_does_not_set_nonblocking_on_the_original_description() {
        let mut fds = [0; 2];
        // SAFETY: fds has space for both fresh pipe descriptors.
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        // SAFETY: pipe2 returned two distinct fresh descriptors owned by this scope.
        let (_reader, writer) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        // SAFETY: writer owns a valid descriptor; F_GETFL only reads its flags.
        let before = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
        let mut sink = ConsoleSink::from_fd(writer.as_raw_fd()).unwrap();
        for _ in 0..100 {
            sink.echo(&[b'x'; 8192]);
        }
        // SAFETY: writer still owns the same valid descriptor.
        let after = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(after, before);
    }
}
