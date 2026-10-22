//! Controlled scheduler contention through the validation runner. `chrt -f 50` needs privilege
//! the unprivileged campaign domain lacks, so when the campaign environment names a runner the
//! contender is requested as a typed `ExecuteSchedulerBurst` instead; manual operator workflows
//! keep the direct `chrt` path.
use std::path::PathBuf;

use stutter_validation_runner::{
    client::{Client, SOCKET_ENV},
    model::{Request, ResponseStatus},
};

pub const CAMPAIGN_ENV: &str = "STUTTER_VALIDATION_CAMPAIGN";
pub const TASK_ENV: &str = "STUTTER_VALIDATION_TASK";
pub const MANIFEST_ENV: &str = "STUTTER_VALIDATION_MANIFEST";

/// Everything the campaign controller exports so a nested suite can request a burst.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerEnv {
    pub socket: PathBuf,
    pub campaign_id: String,
    pub task_id: String,
    pub manifest_sha256: String,
}

impl RunnerEnv {
    /// `None` unless all four variables are present and non-empty.
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        let value = |name| get(name).filter(|value| !value.is_empty());
        Some(Self {
            socket: PathBuf::from(value(SOCKET_ENV)?),
            campaign_id: value(CAMPAIGN_ENV)?,
            task_id: value(TASK_ENV)?,
            manifest_sha256: value(MANIFEST_ENV)?,
        })
    }

    pub fn from_process() -> Option<Self> {
        Self::from_vars(&|name| std::env::var(name).ok())
    }

    /// The typed request for one controlled contender (period fixed by the caller).
    pub fn burst_request(cpu: u32, burst_ms: u64, period_ms: u64, seconds: u64) -> Request {
        Request::ExecuteSchedulerBurst {
            cpu,
            burst_ms,
            period_ms,
            seconds,
        }
    }

    /// Asks the runner to start the contender and returns its pid.
    pub fn start_burst(
        &self,
        cpu: u32,
        burst_ms: u64,
        period_ms: u64,
        seconds: u64,
    ) -> anyhow::Result<u32> {
        let client = Client {
            socket: &self.socket,
            campaign_id: &self.campaign_id,
            manifest_sha256: &self.manifest_sha256,
        };
        let response = client.send(
            &self.task_id,
            Self::burst_request(cpu, burst_ms, period_ms, seconds),
        )?;
        anyhow::ensure!(
            response.status == ResponseStatus::Ok,
            "the runner refused the scheduler burst: {:?}",
            response.error
        );
        response
            .data
            .and_then(|data| data["pid"].as_u64())
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| anyhow::anyhow!("the runner did not report the burst pid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(entries: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map = entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn the_runner_path_needs_every_variable_and_the_manual_path_is_untouched() {
        let full = [
            (SOCKET_ENV, "/run/stutter-validation/runner.sock"),
            (CAMPAIGN_ENV, "core-1"),
            (TASK_ENV, "frame-score"),
            (MANIFEST_ENV, "abc"),
        ];
        let env = RunnerEnv::from_vars(&vars(&full)).unwrap();
        assert_eq!(env.task_id, "frame-score");
        assert_eq!(
            env.socket,
            std::path::Path::new("/run/stutter-validation/runner.sock")
        );
        // Dropping any single variable (or leaving it empty) falls back to direct behavior.
        for missing in 0..full.len() {
            let partial = full
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != missing)
                .map(|(_, e)| *e)
                .collect::<Vec<_>>();
            assert!(
                RunnerEnv::from_vars(&vars(&partial)).is_none(),
                "missing {missing}"
            );
        }
        assert!(
            RunnerEnv::from_vars(&vars(&[
                (SOCKET_ENV, ""),
                (CAMPAIGN_ENV, "c"),
                (TASK_ENV, "t"),
                (MANIFEST_ENV, "m")
            ]))
            .is_none()
        );
        assert!(RunnerEnv::from_vars(&|_| None).is_none());
    }

    #[test]
    fn the_burst_request_carries_only_typed_numbers() {
        assert_eq!(
            RunnerEnv::burst_request(2, 4, 20, 40),
            Request::ExecuteSchedulerBurst {
                cpu: 2,
                burst_ms: 4,
                period_ms: 20,
                seconds: 40
            }
        );
    }
}
