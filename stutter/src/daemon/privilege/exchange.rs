//! Digest-bound prepared transaction exchange and timeout.
use super::*;

static COMMIT_TIMEOUT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(15_000);

pub(super) fn commit_timeout() -> Duration {
    Duration::from_millis(COMMIT_TIMEOUT_MS.load(std::sync::atomic::Ordering::Relaxed))
}

/// How long a prepared apply waits for the controller's commit; tests shorten it.
#[cfg(test)]
pub(crate) fn set_commit_timeout_for_tests(timeout: Duration) {
    COMMIT_TIMEOUT_MS.store(
        u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// One two-phase exchange on a connection: send `Prepared`, then wait for exactly one
/// `CommitPrepared`. Anything else (abort, wrong digest, silence, a closed connection) aborts the
/// apply before the first mutable write.
pub(super) struct CommitExchange<'a> {
    stream: &'a UnixStream,
    timeout: Duration,
}

impl<'a> CommitExchange<'a> {
    pub(super) fn new(stream: &'a UnixStream, timeout: Duration) -> Self {
        Self { stream, timeout }
    }

    pub(super) fn prepare_and_wait(&mut self, token: &RollbackToken) -> anyhow::Result<()> {
        let digest = rollback_snapshot_digest(token);
        let mut writer = self.stream;
        let prepared = PrivilegedWorkerResponse::Prepared {
            rollback: token.clone(),
            snapshot_digest: digest.clone(),
        };
        serde_json::to_writer(&mut writer, &prepared)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        self.stream.set_read_timeout(Some(self.timeout))?;
        let mut reader = BufReader::new(self.stream.try_clone()?);
        let mut line = String::new();
        let read = reader.read_line(&mut line);
        match read {
            Ok(0) => Err(PrivilegedWorkerError::CommitRejected {
                reason: "the controller closed the connection after the prepare".to_owned(),
            }
            .into()),
            Ok(_) => match serde_json::from_str::<PrivilegedWorkerRequest>(&line) {
                Ok(PrivilegedWorkerRequest::CommitPrepared {
                    commit: true,
                    snapshot_digest,
                }) if snapshot_digest == digest => Ok(()),
                Ok(PrivilegedWorkerRequest::CommitPrepared { commit: true, .. }) => {
                    Err(PrivilegedWorkerError::CommitRejected {
                        reason: "the commit names a different snapshot than the one prepared"
                            .to_owned(),
                    }
                    .into())
                }
                Ok(PrivilegedWorkerRequest::CommitPrepared { commit: false, .. }) => {
                    Err(PrivilegedWorkerError::CommitRejected {
                        reason: "the controller aborted the prepared apply".to_owned(),
                    }
                    .into())
                }
                _ => Err(PrivilegedWorkerError::CommitRejected {
                    reason: "the answer to the prepare was not a commit".to_owned(),
                }
                .into()),
            },
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Err(PrivilegedWorkerError::CommitExpired {
                    timeout_ms: self.timeout.as_millis(),
                }
                .into())
            }
            Err(error) => Err(error.into()),
        }
    }
}
