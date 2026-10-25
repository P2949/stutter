//! The production benefit harness: one matched pair per privileged-runner request. The runner
//! launches the `affinity-benefit` live suite as the campaign user with its fixed capability set;
//! the suite drives the production live controller and writes the populations it compared.
use std::{fs, path::PathBuf};

use serde_json::Value;
use stutter::api::autotune::ObjectiveKind;
use stutter_validation_runner::{
    client::Client,
    model::{BenefitScenario, BenefitSpec, ErrorCode, LiveSuite, Request, ResponseStatus},
};

use super::{
    benefit::{Order, PairResult, RunnableSummaryRecord, Scenario},
    graph::Stage,
    runner::{BenefitHarness, HarnessError, StageContext},
};
use crate::validation_campaign::model::{BlockReason, FailureClass};

pub struct RunnerHarness {
    pub socket: PathBuf,
}

/// Rejections that mean "this campaign may not do that", which retrying cannot fix.
fn is_privilege_denial(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::UnauthorizedPeer
            | ErrorCode::UnknownCampaign
            | ErrorCode::ManifestMismatch
            | ErrorCode::ActionNotPermitted
            | ErrorCode::TaskNotPermitted
            | ErrorCode::SchemaMismatch
            | ErrorCode::Cancelled
    )
}

/// The next unused measurement attempt number of `stage` (the runner derives the directory).
fn next_attempt(context: &StageContext<'_>, stage: Stage) -> u32 {
    let base = context.dir.0.join("attempts").join(stage.as_str());
    fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .max()
        .map_or(1, |highest| highest + 1)
}

fn scenario_of(scenario: Scenario) -> BenefitScenario {
    match scenario {
        Scenario::Beneficial => BenefitScenario::Beneficial,
        Scenario::Neutral => BenefitScenario::Neutral,
        Scenario::Regression => BenefitScenario::Regression,
    }
}

fn summary(value: &Value) -> Option<RunnableSummaryRecord> {
    Some(RunnableSummaryRecord {
        p99_ns: value["p99_ns"].as_u64()?,
        samples: value["samples"].as_u64()?,
        over_5ms: value["over_5ms"].as_u64()?,
    })
}

/// Reads the pair the suite wrote; `None` when it is incomplete.
pub fn read_pair(
    raw: &std::path::Path,
    scenario: Scenario,
    pair: usize,
    order: Order,
) -> Option<PairResult> {
    let value: Value = serde_json::from_slice(&fs::read(raw.join("pair.json")).ok()?).ok()?;
    Some(PairResult {
        scenario,
        pair,
        order,
        baseline: summary(&value["baseline"])?,
        candidate: summary(&value["candidate"])?,
        placement_observed: value["placement_observed"].as_bool()?,
        guardrails_ok: value["guardrails_ok"].as_bool()?,
        manager_kept: value["manager_kept"].as_bool()?,
    })
}

impl BenefitHarness for RunnerHarness {
    fn measure(
        &self,
        context: &StageContext<'_>,
        _objective: ObjectiveKind,
        scenario: Scenario,
        pair: usize,
        order: Order,
    ) -> Result<PairResult, HarnessError> {
        let stage = Stage::BenefitCalibration;
        let attempt = next_attempt(context, stage);
        let client = Client {
            socket: &self.socket,
            campaign_id: &context.state.campaign_id,
            manifest_sha256: &context.state.manifest_sha256,
        };
        let request = Request::ExecuteLiveSuite {
            suite: LiveSuite::AffinityBenefit,
            attempt,
            seconds: Some(context.manifest.benefit.window_seconds),
            rounds: None,
            benefit: Some(BenefitSpec {
                scenario: scenario_of(scenario),
                pair: pair as u32,
                baseline_first: order == Order::BaselineFirst,
            }),
        };
        let response = client.send(stage.as_str(), request).map_err(|error| {
            HarnessError::Blocked(BlockReason {
                class: FailureClass::BlockedPrivilege,
                detail: format!(
                    "the validation runner at {} is unreachable: {error}",
                    self.socket.display()
                ),
            })
        })?;
        if response.status != ResponseStatus::Ok {
            let error = response.error.expect("a refused response carries an error");
            let message = format!("runner refused: {:?}: {}", error.code, error.message);
            return Err(if is_privilege_denial(error.code) {
                HarnessError::Blocked(BlockReason {
                    class: FailureClass::BlockedPrivilege,
                    detail: message,
                })
            } else {
                HarnessError::Retryable(message)
            });
        }
        let raw = context.dir.attempt_dir(stage.as_str(), attempt).join("raw");
        let result: Value = fs::read(raw.join("result.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| HarnessError::Retryable("the pair wrote no result.json".to_owned()))?;
        if result["status"] != "PASS" {
            return Err(HarnessError::Retryable(format!(
                "pair {scenario:?}/{pair} reported {}: {}",
                result["status"].as_str().unwrap_or("no status"),
                result["error"].as_str().unwrap_or("no error recorded")
            )));
        }
        read_pair(&raw, scenario, pair, order)
            .ok_or_else(|| HarnessError::Defect("the pair record is incomplete".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_pair_file_becomes_a_pair_result() {
        let dir = std::env::temp_dir().join(format!("stutter-harness-pair-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("pair.json"),
            br#"{"baseline":{"p99_ns":9000000,"samples":2500,"over_5ms":40},
            "candidate":{"p99_ns":6000000,"samples":2600,"over_5ms":10},
            "placement_observed":true,"guardrails_ok":true,"manager_kept":true}"#,
        )
        .unwrap();
        let pair = read_pair(&dir, Scenario::Beneficial, 4, Order::CandidateFirst).unwrap();
        assert_eq!(pair.baseline.p99_ns, 9_000_000);
        assert_eq!(pair.candidate.samples, 2_600);
        assert!(pair.placement_observed && pair.manager_kept);
        // An incomplete population is not a pair.
        fs::write(dir.join("pair.json"), br#"{"baseline":{"p99_ns":1}}"#).unwrap();
        assert!(read_pair(&dir, Scenario::Beneficial, 4, Order::CandidateFirst).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
