//! Exact-release acceptance. The evidence that certifies a release commit cannot be stored
//! inside that commit (committing the evidence changes the commit), so acceptance consumes
//! external bundles that are verified on the spot. The committed `docs/evidence/index.json`
//! is the historical project catalogue and never satisfies this gate.
use std::{collections::BTreeSet, path::PathBuf};

use anyhow::{Context, ensure};
use stutter::api::capabilities::{Capability, CapabilityTier};

use crate::evidence_bundle::verified_index_entry;

/// Fails closed unless every supplied bundle verifies as a PASS for exactly `commit` and
/// together they cover every release suite (`required_release_evidence`) of every Supported
/// capability; historical graduation evidence (`required_evidence`) is never re-demanded here. Any bad bundle
/// is an error regardless of order; several distinct captures of one suite are allowed, the
/// same capture twice is not. Returns the accepted suites.
pub(super) fn check_acceptance(
    capabilities: &[Capability],
    commit: &str,
    bundles: &[PathBuf],
) -> anyhow::Result<BTreeSet<String>> {
    let supported = capabilities
        .iter()
        .filter(|c| c.tier == CapabilityTier::Supported)
        .collect::<Vec<_>>();
    ensure!(
        !supported.is_empty(),
        "no Supported capability: nothing may be released as supported"
    );
    ensure!(
        !bundles.is_empty(),
        "acceptance needs verified exact-commit bundles (--acceptance-bundle); the committed \
         docs/evidence/index.json is historical and cannot certify the commit that contains it"
    );
    let mut sorted = bundles.to_vec();
    sorted.sort();
    let (mut passing, mut captures) = (BTreeSet::new(), BTreeSet::new());
    for bundle in &sorted {
        let entry = verified_index_entry(bundle)
            .with_context(|| format!("acceptance bundle {} does not verify", bundle.display()))?;
        let suite = entry["suite"].as_str().unwrap_or_default();
        ensure!(
            entry["status"] == "PASS",
            "acceptance bundle {} ({suite}) is {}, not PASS",
            bundle.display(),
            entry["status"]
        );
        ensure!(
            entry["source_commit"] == commit,
            "acceptance bundle {} ({suite}) is for {}, not release commit {commit}",
            bundle.display(),
            entry["source_commit"]
        );
        let capture = entry["capture_id"].as_str().unwrap_or_default().to_owned();
        ensure!(
            captures.insert(capture.clone()),
            "capture {capture} was supplied twice"
        );
        passing.insert(suite.to_owned());
    }
    for capability in supported {
        for suite in capability.required_release_evidence {
            ensure!(
                passing.contains(*suite),
                "{} requires {suite} PASS evidence for exactly {commit}",
                capability.id
            );
        }
    }
    Ok(passing)
}
