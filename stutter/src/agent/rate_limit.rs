//! Agent request rate limiting.

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::AgentRateLimiter;

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
