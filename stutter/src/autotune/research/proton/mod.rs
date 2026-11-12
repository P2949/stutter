//! Read-only inventory of the six pinned Proton sources. Paths stay process-local and never enter
//! intent or report serialization; identities bind provenance and the exact entrypoint manifests.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{ProtonSource, ResearchTool, SteamUpdateStatus};

mod custom;
mod steam;
mod steam_client;
mod tool;

#[cfg(test)]
use custom::{custom_candidates, supports_x86_64_v3, x86_64_v3_flag_sets_supported};
pub(crate) use steam::inspect_proton_inventory;
#[cfg(test)]
use steam::{inspect_valve_stable, merge_valve_candidates};
#[cfg(test)]
use steam_client::stable_proton_version;
#[cfg(test)]
use steam_client::{inspect_steam_process, steam_webhelper_binding};
use tool::{digest_ok, valid_steam_branch};
#[cfg(test)]
use tool::{validate_toolmanifests, version_line};

const MAX_LAB_MANIFEST: u64 = 1024 * 1024;
const MAX_TOOL_FILE: u64 = 32 * 1024 * 1024;
const MAX_STEAM_MANIFEST: u64 = 1024 * 1024;
const MAX_STEAM_MANIFEST_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STEAM_MANIFEST_COUNT: usize = 4096;
const MAX_STEAM_CLIENT_FILE: u64 = 8 * 1024 * 1024;
const MAX_PROC_CMDLINE: usize = 16 * 1024;
const EXPERIMENTAL_APP_ID: u32 = 1_493_710;
const STEAM_RUNTIME_APP_ID: &str = "4183110";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtonBlockReason {
    NotInstalled,
    Updating,
    AmbiguousStableBuild,
    AmbiguousInstallations,
    UnsupportedCpuIsa,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ProtonEligibility {
    Eligible,
    Blocked(ProtonBlockReason),
}

/// Serialized evidence deliberately excludes machine-specific install paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtonCandidate {
    pub source: ProtonSource,
    pub version: Option<String>,
    pub tool: Option<ResearchTool>,
    pub eligibility: ProtonEligibility,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl ProtonCandidate {
    fn blocked(source: ProtonSource, reason: ProtonBlockReason) -> Self {
        Self {
            source,
            version: None,
            tool: None,
            eligibility: ProtonEligibility::Blocked(reason),
            path: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtonInventory {
    pub schema_version: u32,
    pub candidates: Vec<ProtonCandidate>,
    pub steam_client: Option<SteamClientIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteamClientIdentity {
    pub branch: String,
    pub steamrt64_build_id: u64,
    pub steamrt64_manifest_sha256: String,
    pub installed_index_sha256: String,
    pub identity_sha256: String,
    pub process: Option<SteamProcessIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteamProcessIdentity {
    pub steam_pid: u32,
    pub steam_start_ticks: u64,
    pub webhelper_pid: u32,
    pub webhelper_start_ticks: u64,
    /// Legacy wire field: a nonzero webhelper Steam ID is an account hint, not proof of
    /// authentication, entitlement or the account's effective launch settings.
    pub logged_in: bool,
}

impl SteamClientIdentity {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            valid_steam_branch(&self.branch)
                && self.steamrt64_build_id != 0
                && digest_ok(&self.steamrt64_manifest_sha256)
                && digest_ok(&self.installed_index_sha256)
                && digest_ok(&self.identity_sha256),
            "invalid Steam client identity"
        );
        if let Some(process) = &self.process {
            anyhow::ensure!(
                process.steam_pid != 0
                    && process.steam_start_ticks != 0
                    && process.webhelper_pid != 0
                    && process.webhelper_start_ticks != 0
                    && process.steam_pid != process.webhelper_pid,
                "invalid Steam process identity"
            );
        }
        Ok(())
    }
}

impl ProtonInventory {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1,
            "unsupported Proton inventory schema"
        );
        let sources: BTreeSet<_> = self.candidates.iter().map(|item| item.source).collect();
        anyhow::ensure!(
            sources.len() == 6 && self.candidates.len() == 6,
            "incomplete Proton inventory"
        );
        for candidate in &self.candidates {
            match candidate.eligibility {
                ProtonEligibility::Eligible => anyhow::ensure!(
                    candidate.tool.is_some() && candidate.path.is_some(),
                    "eligible Proton lacks a resolved installation"
                ),
                ProtonEligibility::Blocked(ProtonBlockReason::NotInstalled) => anyhow::ensure!(
                    candidate.tool.is_none() && candidate.path.is_none(),
                    "missing Proton unexpectedly has an installation"
                ),
                ProtonEligibility::Blocked(ProtonBlockReason::AmbiguousStableBuild) => {
                    anyhow::ensure!(
                        candidate.source == ProtonSource::ValveStable
                            && candidate.tool.is_some()
                            && candidate.path.is_none(),
                        "ambiguous Proton Stable inventory is malformed"
                    )
                }
                ProtonEligibility::Blocked(ProtonBlockReason::AmbiguousInstallations) => {
                    anyhow::ensure!(
                        matches!(
                            candidate.source,
                            ProtonSource::ValveStable | ProtonSource::ValveExperimental
                        ) && candidate.tool.is_some()
                            && candidate.path.is_none(),
                        "ambiguous Valve Proton inventory is malformed"
                    )
                }
                ProtonEligibility::Blocked(_) => anyhow::ensure!(
                    candidate.tool.is_some() && candidate.path.is_some(),
                    "blocked installed Proton lacks its identity"
                ),
            }
        }
        if let Some(client) = &self.steam_client {
            client.validate()?;
        }
        Ok(())
    }

    /// Installation directory of an eligible candidate whose frozen identity is exactly `tool`.
    pub(crate) fn eligible_path(&self, tool: &ResearchTool) -> Option<&Path> {
        self.candidates
            .iter()
            .find(|item| {
                item.eligibility == ProtonEligibility::Eligible && item.tool.as_ref() == Some(tool)
            })
            .and_then(|item| item.path.as_deref())
    }

    pub fn tools(&self) -> Vec<ResearchTool> {
        self.candidates
            .iter()
            .filter(|item| item.eligibility == ProtonEligibility::Eligible)
            .filter_map(|item| item.tool.clone())
            .collect()
    }
}

#[cfg(test)]
#[path = "../proton_tests.rs"]
mod tests;
