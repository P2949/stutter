//! One SIGINT listener for a whole select loop.
//!
//! `tokio::signal::ctrl_c()` registers a listener when it is first polled and drops it with the
//! future, and a listener only sees signals that arrive after it exists. Called inside a loop's
//! `select!` it therefore leaves a gap while any other arm's handler runs, and an interrupt landing
//! in that gap is lost: the controller then ran on after a Ctrl-C and left its action applied.
//! `InterruptWatch` creates the listener once, before the loop; an interrupt delivered while the
//! caller was busy is returned by the next [`InterruptWatch::interrupted`].
use tokio::signal::unix::{Signal, SignalKind, signal};

pub(crate) struct InterruptWatch(Signal);

impl InterruptWatch {
    /// Installs the SIGINT handler now, not at the first poll.
    pub(crate) fn new() -> std::io::Result<Self> {
        signal(SignalKind::interrupt()).map(Self)
    }

    /// Completes when an interrupt has arrived since the last one was returned. Cancel-safe.
    pub(crate) async fn interrupted(&mut self) {
        if self.0.recv().await.is_none() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::InterruptWatch;

    /// One test, run in order: both halves raise SIGINT in this process, and every live listener
    /// would see either of them.
    #[tokio::test]
    async fn an_interrupt_that_arrives_while_the_caller_is_busy_is_not_lost() {
        let mut watch = InterruptWatch::new().unwrap();
        // SAFETY: raise(3) has no memory effects; the handler installed above consumes the signal.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        // Nothing is waiting on the watch while the caller is busy elsewhere...
        tokio::time::sleep(Duration::from_millis(50)).await;
        // ...and the next wait still sees the interrupt.
        tokio::time::timeout(Duration::from_secs(2), watch.interrupted())
            .await
            .expect("the interrupt delivered while busy must be returned");
        // It is consumed: with no further signal the next wait does not complete.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), watch.interrupted())
                .await
                .is_err()
        );

        // The pattern `ctrl_c()` had inside a loop: a listener created after the interrupt
        // misses it, which is how a controller ran on after Ctrl-C.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let late = tokio::time::timeout(Duration::from_millis(200), tokio::signal::ctrl_c()).await;
        assert!(late.is_err(), "a per-iteration ctrl_c() loses it");
        // The watch, alive throughout, did not.
        tokio::time::timeout(Duration::from_secs(2), watch.interrupted())
            .await
            .expect("the long-lived watch keeps the interrupt");
    }
}
