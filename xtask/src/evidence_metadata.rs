//! Provenance shared by every producer of canonical evidence (live acceptance and the soak): the
//! clean-source/binary/harness/environment record that `evidence-export` copies into a bundle, and
//! the `result.json` writer that refuses a result whose identity disagrees with it.
use std::{fs, path::Path, process::Command};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

/// Short-run production-controller acceptance of the graduated operations.
pub(crate) const SUITE_AUTOTUNE_SMOKE: &str = "live_autotune_smoke";
/// The coverage-qualified real-system bounded-controller soak (minimum 8 h dwell, complete
/// deterministic cycles, repeated coverage of every fault kind, zero violations) that graduates
/// the bounded controller.
pub(crate) const SUITE_AUTOTUNE_SOAK: &str = "live_autotune_soak";

pub(crate) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Where the source identity comes from. A live suite reads the checkout it runs in; a soak is
/// bound to the commit it was initialised at, which may no longer be `HEAD` when it is verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source<'a> {
    Checkout,
    Bound { commit: &'a str, dirty: bool },
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    let output = Command::new("sha256sum").arg("--").arg(path).output()?;
    ensure!(output.status.success(), "hashing {} failed", path.display());
    Ok(String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .context("missing digest")?
        .to_owned())
}

/// The environment record of one capture. It is returned even when the source is dirty or the
/// binaries do not match; `require_clean` is the gate that refuses to publish such a capture.
pub(crate) fn capture(
    root: &Path,
    binary: &Path,
    suite: &str,
    evidence_kind: &str,
    source: Source<'_>,
) -> anyhow::Result<Value> {
    let run = |program: &str, argv: &[&str]| -> anyhow::Result<String> {
        let output = Command::new(program)
            .args(argv)
            .current_dir(root)
            .output()?;
        ensure!(output.status.success(), "metadata command {program} failed");
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    let captured_unix_ms = unix_ms();
    let (commit, short, dirty) = match source {
        Source::Checkout => (
            run("git", &["rev-parse", "HEAD"])?,
            run("git", &["rev-parse", "--short", "HEAD"])?,
            !run("git", &["status", "--porcelain"])?.is_empty(),
        ),
        Source::Bound { commit, dirty } => {
            run("git", &["cat-file", "-e", &format!("{commit}^{{commit}}")])
                .context("the bound source commit is not in this repository")?;
            (
                commit.to_owned(),
                commit.chars().take(8).collect::<String>(),
                dirty,
            )
        }
    };
    let version = Command::new(binary).arg("--version").output()?;
    ensure!(version.status.success(), "binary version command failed");
    let version = String::from_utf8(version.stdout)?.trim().to_owned();
    let source_matches_binary =
        crate::validation_campaign::repository::reports_revision(&version, &commit);
    let linked_library_version = stutter::api::build_version();
    let source_matches_harness = version == format!("stutter {linked_library_version}");
    let harness_hash = sha256_of(&std::env::current_exe()?)?;
    // Keep the digest only: absolute checkout and user paths are not provenance.
    let digest = sha256_of(binary)?;
    let os = fs::read_to_string("/etc/os-release")?
        .lines()
        .filter(|line| line.starts_with("ID=") || line.starts_with("VERSION_ID="))
        .collect::<Vec<_>>()
        .join("\n");
    let cpu_model = fs::read_to_string("/proc/cpuinfo")?
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name").and_then(|tail| {
                tail.split_once(':')
                    .map(|(_, value)| value.trim().to_owned())
            })
        });
    Ok(
        json!({"schema_version":1,"commit":commit,"binary_version":version,"source_matches_binary":source_matches_binary,
        "source_dirty":dirty,"binary_sha256":digest,"kernel_release":fs::read_to_string("/proc/sys/kernel/osrelease")?.trim(),
        "proc_version":fs::read_to_string("/proc/version")?.trim(),"distribution":os,"cpu_model":cpu_model,
        "cpu_online":fs::read_to_string("/sys/devices/system/cpu/online")?.trim(),
        "rust":run("rustc", &["-Vv"] )?,"cargo_lock_sha256":run("sha256sum", &["Cargo.lock"] )?.split_whitespace().next(),
        "evidence_kind":evidence_kind,"suite":suite,"linked_library_version":linked_library_version,"source_matches_harness":source_matches_harness,"harness_sha256":harness_hash,
        "captured_unix_ms":captured_unix_ms,"capture_id":format!("{suite}-{short}-{captured_unix_ms}"),
        "gpu_relevant":false,
        "artifact_schema":{"evidence_result":1,"session":stutter::api::recorder::SESSION_SCHEMA_VERSION.get(),
            "controller_journal":stutter::api::autotune::controller_journal::CONTROLLER_JOURNAL_SCHEMA_VERSION,"restore_state":1}}),
    )
}

/// Refuses a capture that could not become publishable evidence.
pub(crate) fn require_clean(environment: &Value) -> anyhow::Result<()> {
    ensure!(
        environment["source_dirty"] == false,
        "acceptance requires a clean committed source tree"
    );
    ensure!(
        environment["source_matches_binary"] == true,
        "binary revision differs from source; rebuild stutter before acceptance"
    );
    ensure!(
        environment["source_matches_harness"] == true,
        "xtask linked library differs from source/CLI; rebuild both binaries"
    );
    Ok(())
}

/// Writes `environment.json` (kept even for a refused capture, so the refusal is inspectable).
pub(crate) fn write_environment(out: &Path, environment: &Value) -> anyhow::Result<()> {
    fs::write(
        out.join("environment.json"),
        serde_json::to_vec_pretty(environment)?,
    )?;
    Ok(())
}

/// Writes `result.json` with the suite identity injected, refusing to publish a result whose
/// suite differs from the already-written `environment.json`.
pub(crate) fn write_result(out: &Path, suite: &str, mut result: Value) -> anyhow::Result<()> {
    result["suite"] = json!(suite);
    result["finished_unix_ms"] = json!(unix_ms());
    if let Ok(bytes) = fs::read(out.join("environment.json")) {
        let environment: Value = serde_json::from_slice(&bytes)?;
        result["capture_id"] = environment["capture_id"].clone();
        result["captured_unix_ms"] = environment["captured_unix_ms"].clone();
        ensure!(
            environment["evidence_kind"] == result["evidence_kind"],
            "environment.json evidence_kind differs from the result"
        );
        ensure!(
            environment["suite"] == result["suite"],
            "environment.json suite {} differs from result suite {}",
            environment["suite"],
            result["suite"]
        );
    }
    fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soak_suites_belong_to_the_canonical_vocabulary() {
        for suite in [SUITE_AUTOTUNE_SMOKE, SUITE_AUTOTUNE_SOAK] {
            assert!(crate::evidence_bundle::KNOWN_SUITES.contains(&suite));
        }
    }
}
