//! Best-effort console echo must not block bounded capture on a full pipe or socket.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileTypeExt, OpenOptionsExt},
    },
};

/// Longest a single chunk may wait for a slow console before the sink is declared stalled.
const STALL_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

pub(super) enum ConsoleSink {
    File(File, bool),
    Socket(File, bool),
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
            return Some(Self::Socket(file, false));
        }
        if kind.is_file() {
            // Preserve the original shared file offset/append semantics for regular logs.
            return Some(Self::File(file, false));
        }
        if kind.is_fifo() || kind.is_char_device() {
            // Reopening procfd creates an independent open description: O_NONBLOCK must not
            // change the original stdout/stderr flags used by the rest of the application.
            let sink = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(format!("/proc/self/fd/{}", file.as_raw_fd()))
                .ok()?;
            return Some(Self::File(sink, false));
        }
        None
    }

    /// Echoes `bytes`, waiting briefly while the console is full. Returns how many bytes were
    /// written. A sink that stays stalled past `STALL_BUDGET` is abandoned so capture never blocks.
    pub(super) fn echo(&mut self, bytes: &[u8]) -> usize {
        let (file, stalled, socket) = match self {
            Self::File(file, stalled) => (file, stalled, false),
            Self::Socket(file, stalled) => (file, stalled, true),
        };
        if *stalled {
            return 0;
        }
        let started = std::time::Instant::now();
        let mut done = 0;
        while done < bytes.len() {
            let rest = &bytes[done..];
            let written = if socket {
                // SAFETY: file owns a socket fd and rest is valid for the supplied length.
                let sent = unsafe {
                    libc::send(
                        file.as_raw_fd(),
                        rest.as_ptr().cast(),
                        rest.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                usize::try_from(sent).map_err(|_| std::io::Error::last_os_error())
            } else {
                file.write(rest)
            };
            match written {
                Ok(0) => break,
                Ok(count) => done += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= STALL_BUDGET {
                        *stalled = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(_) => {
                    *stalled = true;
                    break;
                }
            }
        }
        done
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
