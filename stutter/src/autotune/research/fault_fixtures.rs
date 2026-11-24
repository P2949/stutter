//! The producer of the destructive fault classes' fixture cases (`fault_matrix::FIXTURE_TESTS`): it runs each class's
//! fixture through a runner, stores the raw record of that run beside the study as the case's evidence, and returns the
//! case the matrix receipt carries. The benign classes need an operator's physical injection and are not produced here.
//!
//! A fixture that does not pass, a fixture the runner cannot find, or a record that cannot be stored yields a failed
//! case or an error, never a pass.

use std::fs::File;

use serde_json::json;

use super::{EVIDENCE_DIR, FIXTURE_TESTS, FaultCase, Mode, Outcome};

/// What running one fixture test showed.
pub struct FixtureRun {
    pub passed: bool,
    /// The runner's own output, kept verbatim in the evidence record.
    pub output: String,
}

/// Run every destructive class's fixture through `run` (given the test's name) and store the evidence of each beside
/// `study` for the binary `binary_sha256`. Returns one case per class, in `FIXTURE_TESTS` order.
pub fn produce(
    study: &std::path::Path,
    binary_sha256: &str,
    now: u64,
    mut run: impl FnMut(&str) -> Option<FixtureRun>,
) -> anyhow::Result<Vec<FaultCase>> {
    std::fs::create_dir_all(study.join(EVIDENCE_DIR))?;
    let dir = File::from(crate::syscall::open_dir_no_symlinks(
        &study.join(EVIDENCE_DIR),
    )?);
    let mut cases = Vec::new();
    for (class, test) in FIXTURE_TESTS {
        let result = run(test);
        let outcome = if result.as_ref().is_some_and(|run| run.passed) {
            Outcome::Passed
        } else {
            Outcome::Failed
        };
        let record = json!({
            "class": class,
            "mode": "fixture",
            "outcome": if outcome == Outcome::Passed { "passed" } else { "failed" },
            "binary_sha256": binary_sha256,
            "test": test,
            "found": result.is_some(),
            "output": result.map(|run| run.output).unwrap_or_default(),
            "observed_unix": now,
        });
        let bytes = serde_json::to_vec(&record)?;
        let digest = stutter_core::hash::sha256_hex(&bytes);
        match crate::autotune::research::report_store::write_new(
            &dir,
            &format!("{digest}.json"),
            &bytes,
        ) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) => {}
            Err(error) => return Err(error),
        }
        cases.push(FaultCase {
            class: class.to_owned(),
            mode: Mode::Fixture,
            outcome,
            evidence_sha256: digest,
            observed_unix: now,
        });
    }
    Ok(cases)
}

/// Join the fixture cases with the operator's physical cases into the matrix receipt, check that the receipt attests and that every
/// case's evidence reopens (the same checks G6 makes), and only then write it beside the study as `fault-matrix-<now>.json`.
/// Nothing is written for a matrix that would not attest.
pub fn assemble(
    study: &std::path::Path,
    binary_sha256: &str,
    now: u64,
    fixtures: Vec<FaultCase>,
    physical: Vec<FaultCase>,
) -> anyhow::Result<super::FaultMatrixReceipt> {
    let receipt = super::FaultMatrixReceipt {
        schema: 1,
        binary_sha256: binary_sha256.to_owned(),
        captured_unix: now,
        cases: fixtures.into_iter().chain(physical).collect(),
    };
    super::attested(&receipt, binary_sha256, now).map_err(|reason| anyhow::anyhow!(reason))?;
    super::artifacts_verified(&receipt, study).map_err(|reason| anyhow::anyhow!(reason))?;
    let dir = File::from(crate::syscall::open_dir_no_symlinks(study)?);
    crate::autotune::research::report_store::write_new(
        &dir,
        &format!("fault-matrix-{now}.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(receipt)
}

/// A runner over the text of a `cargo test` run: a fixture passed when its line ends `ok`, failed when the log says anything
/// else about it, and is missing when the log never names it. The matching line is kept as the evidence output.
pub fn runner_from_log(log: &str) -> impl FnMut(&str) -> Option<FixtureRun> + '_ {
    move |test| {
        let line = log
            .lines()
            .find(|line| line.starts_with("test ") && line.contains(&format!("::{test} ... ")))?;
        Some(FixtureRun {
            passed: line.ends_with(" ... ok"),
            output: line.to_owned(),
        })
    }
}

#[cfg(test)]
#[path = "fault_fixtures_tests.rs"]
mod tests;
