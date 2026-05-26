//! Agent request rate limiting.

use super::*;

#[derive(Debug)]
pub(crate) struct AgentRateLimiter {
    max_requests: usize,
    window: Duration,
    // Only guards the rolling timestamp queue for a single rate-limit decision.
    accepted: Mutex<VecDeque<Instant>>,
}

impl Default for AgentRateLimiter {
    fn default() -> Self {
        Self {
            max_requests: DEFAULT_AGENT_RATE_LIMIT_REQUESTS,
            window: DEFAULT_AGENT_RATE_LIMIT_WINDOW,
            accepted: Mutex::new(VecDeque::new()),
        }
    }
}

impl AgentRateLimiter {
    #[cfg(test)]
    pub(crate) fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            accepted: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) async fn accept(&self, now: Instant) -> bool {
        let mut accepted = self.accepted.lock().await;
        while accepted
            .front()
            .is_some_and(|previous| now.duration_since(*previous) >= self.window)
        {
            accepted.pop_front();
        }

        if accepted.len() >= self.max_requests {
            return false;
        }

        accepted.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::AgentRateLimiter;

    #[tokio::test]
    async fn rate_limiter_rejects_until_window_expires() {
        let limiter = AgentRateLimiter::new(2, Duration::from_secs(10));
        let now = Instant::now();

        assert!(limiter.accept(now).await);
        assert!(limiter.accept(now + Duration::from_secs(1)).await);
        assert!(!limiter.accept(now + Duration::from_secs(2)).await);
        assert!(limiter.accept(now + Duration::from_secs(11)).await);
    }
}
