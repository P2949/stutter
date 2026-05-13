pub use crate::daemon::policy::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_module_reexports_daemon_policy_types() {
        let policy = DaemonPolicy::observe(ActionSource::Test);

        assert_eq!(policy.mode, DaemonMode::Observe);
        assert_eq!(DaemonMode::ApplyLowRisk.to_string(), "apply-low-risk");
    }
}
