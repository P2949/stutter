//! Bounded merged child logs. Pipes keep a noisy child from filling the runner's filesystem;
//! nonblocking reads also let cleanup finish when a detached descendant retains a writer.
use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::AsRawFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(5);
const DRAIN_DEADLINE: Duration = Duration::from_millis(250);

struct State {
    file: File,
    remaining: usize,
    dropped: u64,
    error: Option<String>,
}

pub(crate) struct BoundedLog {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    readers: Vec<JoinHandle<io::Result<bool>>>,
}

fn nonblocking(reader: &impl AsRawFd) -> io::Result<()> {
    let fd = reader.as_raw_fd();
    // SAFETY: reader owns a valid pipe fd and F_GETFL reads its flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this pipe's read description is private to the runner, so no caller flags change.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl BoundedLog {
    pub(crate) fn start(
        file: File,
        stdout: std::process::ChildStdout,
        stderr: std::process::ChildStderr,
        cap: usize,
    ) -> io::Result<Self> {
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let mut log = Self {
            state: Arc::new(Mutex::new(State {
                file,
                remaining: cap,
                dropped: 0,
                error: None,
            })),
            stop: Arc::new(AtomicBool::new(false)),
            readers: Vec::new(),
        };
        log.reader(stdout)?;
        log.reader(stderr)?;
        Ok(log)
    }

    fn reader(&mut self, mut reader: impl Read + Send + 'static) -> io::Result<()> {
        let state = self.state.clone();
        let stop = self.stop.clone();
        self.readers.push(thread::Builder::new().spawn(move || {
            let mut buffer = [0_u8; 8192];
            while !stop.load(Ordering::Acquire) {
                let count = match reader.read(&mut buffer) {
                    Ok(0) => return Ok(true),
                    Ok(count) => count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(POLL);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let mut state = state.lock().map_err(|_| io::Error::other("log poisoned"))?;
                let keep = state.remaining.min(count);
                state.remaining -= keep;
                state.dropped = state.dropped.saturating_add((count - keep) as u64);
                if state.error.is_none()
                    && let Err(error) = state.file.write_all(&buffer[..keep])
                {
                    state.error = Some(error.to_string());
                }
            }
            Ok(false)
        })?);
        Ok(())
    }

    /// The log is evidence only when both pipes reached EOF without truncation or I/O errors.
    pub(crate) fn finish(mut self) -> io::Result<()> {
        let deadline = Instant::now() + DRAIN_DEADLINE;
        while self.readers.iter().any(|reader| !reader.is_finished()) && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        self.stop.store(true, Ordering::Release);
        let mut complete = true;
        let mut failure = None;
        for reader in self.readers.drain(..) {
            match reader.join() {
                Ok(Ok(eof)) => complete &= eof,
                Ok(Err(error)) => failure = Some(error.to_string()),
                Err(_) => failure = Some("log reader panicked".to_owned()),
            }
        }
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("log poisoned"))?;
        if let Some(error) = state.error.as_ref().or(failure.as_ref()) {
            return Err(io::Error::other(format!("incomplete child log: {error}")));
        }
        if !complete || state.dropped != 0 {
            return Err(io::Error::other(format!(
                "incomplete child log: pipes reached EOF={complete}, {} bytes exceeded the immutable cap",
                state.dropped
            )));
        }
        Ok(())
    }
}

impl Drop for BoundedLog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    fn capture(
        script: &str,
        cap: usize,
    ) -> (crate::tests::Scratch, BoundedLog, std::process::Child) {
        let scratch = crate::tests::Scratch::new("bounded-log");
        let file = File::create(scratch.path().join("log")).unwrap();
        let mut child = Command::new("sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let log = BoundedLog::start(
            file,
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
            cap,
        )
        .unwrap();
        (scratch, log, child)
    }

    #[test]
    fn stdout_and_stderr_share_one_cap_and_flood_is_not_success() {
        let (scratch, log, mut child) = capture(
            "head -c 1000000 /dev/zero & head -c 1000000 /dev/zero >&2 & wait",
            16384,
        );
        assert!(child.wait().unwrap().success());
        let error = log.finish().unwrap_err();
        assert!(error.to_string().contains("1983616 bytes"));
        assert_eq!(
            std::fs::metadata(scratch.path().join("log")).unwrap().len(),
            16384
        );
    }

    #[test]
    fn ordinary_complete_output_is_retained() {
        let (scratch, log, mut child) = capture("printf hello; printf world >&2", 100);
        assert!(child.wait().unwrap().success());
        log.finish().unwrap();
        let bytes = std::fs::read(scratch.path().join("log")).unwrap();
        assert!(bytes == b"helloworld" || bytes == b"worldhello");
    }

    #[test]
    fn retained_pipe_writer_cannot_block_log_cleanup() {
        // Keep the exact owned child alive while closing its capture. It is explicitly reaped;
        // this exercises pipe cleanup without leaving a background process or relying on pkill.
        let (_scratch, log, mut child) = capture("exec sleep 60", 100);
        let started = Instant::now();
        let result = log.finish();
        let elapsed = started.elapsed();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(result.unwrap_err().to_string().contains("EOF=false"));
        assert!(elapsed < Duration::from_secs(2));
    }
}
