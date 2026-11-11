use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use super::{
    ProtonBlockReason, ProtonCandidate, ProtonEligibility, ProtonSource,
    tool::{ToolSpec, inspect_tool_directory},
};

fn nix_source_map() -> [(ProtonSource, &'static str, &'static str); 4] {
    [
        (ProtonSource::CachyOs, "cachyos", "proton-cachyos-slr"),
        (
            ProtonSource::CachyOsV3,
            "cachyosV3",
            "proton-cachyos-slr-v3",
        ),
        (ProtonSource::Ge, "ge", "ge-proton-11"),
        (ProtonSource::Em, "em", "proton-em-11"),
    ]
}

pub(super) fn custom_candidates(
    bytes: &[u8],
    manifest_uid: u32,
    store_root: &Path,
) -> anyhow::Result<Vec<ProtonCandidate>> {
    let root: serde_json::Value = serde_json::from_slice(bytes)?;
    let tools = root
        .get("tools")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("Nix Proton manifest lacks tools"))?;
    anyhow::ensure!(
        tools.len() == 4,
        "Nix Proton manifest must contain exactly four tools"
    );
    let expected_keys: BTreeSet<_> = nix_source_map().iter().map(|(_, key, _)| *key).collect();
    anyhow::ensure!(
        tools.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected_keys,
        "unexpected Nix Proton tool set"
    );
    let mut candidates = Vec::new();
    for (source, key, expected_id) in nix_source_map() {
        let tool = &tools[key];
        let value = |field: &str| tool.get(field).and_then(serde_json::Value::as_str);
        let id = value("id").ok_or_else(|| anyhow::anyhow!("Nix tool lacks id"))?;
        let version = value("version").ok_or_else(|| anyhow::anyhow!("Nix tool lacks version"))?;
        let display_name =
            value("displayName").ok_or_else(|| anyhow::anyhow!("Nix tool lacks display name"))?;
        let path = value("path").ok_or_else(|| anyhow::anyhow!("Nix tool lacks path"))?;
        let provenance = tool
            .get("source")
            .ok_or_else(|| anyhow::anyhow!("Nix tool lacks source provenance"))?;
        anyhow::ensure!(
            id == expected_id
                && version.len() <= 128
                && !version.is_empty()
                && !display_name.is_empty()
                && provenance.get("id").and_then(serde_json::Value::as_str) == Some(expected_id)
                && provenance
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    == Some(version)
                && provenance
                    .get("hash")
                    .and_then(serde_json::Value::as_str)
                    .is_some(),
            "Nix Proton provenance mismatch"
        );
        let path = Path::new(path);
        anyhow::ensure!(
            path.starts_with(store_root)
                && path
                    .components()
                    .all(|part| !matches!(part, Component::ParentDir | Component::CurDir)),
            "Nix Proton tool path is outside the store"
        );
        let version_file_identity = if source == ProtonSource::CachyOsV3 {
            version
                .strip_suffix("-v3")
                .ok_or_else(|| anyhow::anyhow!("invalid CachyOS v3 version pin"))?
        } else {
            version
        };
        let mut candidate = inspect_tool_directory(ToolSpec {
            source,
            expected_id,
            expected_version: version_file_identity,
            compatibility_display_name: Some(display_name),
            path,
            owner_uid: manifest_uid,
            immutable: true,
            provenance,
            app_manifest_sha256: None,
        })?;
        candidate.version = Some(version.to_owned());
        if source == ProtonSource::CachyOsV3 && !supports_x86_64_v3() {
            candidate.eligibility =
                ProtonEligibility::Blocked(ProtonBlockReason::UnsupportedCpuIsa);
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

pub(super) fn supports_x86_64_v3() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        let flag_sets_supported =
            std::fs::read_to_string("/proc/cpuinfo")
                .ok()
                .is_some_and(|text| {
                    let cpu_flags: Vec<BTreeSet<&str>> = text
                        .lines()
                        .filter_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| matches!(key.trim(), "flags" | "Features"))
                                .map(|(_, value)| value.split_ascii_whitespace().collect())
                        })
                        .collect();
                    x86_64_v3_flag_sets_supported(&cpu_flags)
                });
        flag_sets_supported
            && std::is_x86_feature_detected!("avx")
            && std::is_x86_feature_detected!("avx2")
            && std::is_x86_feature_detected!("bmi1")
            && std::is_x86_feature_detected!("bmi2")
            && std::is_x86_feature_detected!("fma")
            && std::is_x86_feature_detected!("f16c")
            && std::is_x86_feature_detected!("lzcnt")
            && std::is_x86_feature_detected!("movbe")
            && std::is_x86_feature_detected!("popcnt")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

pub(super) fn x86_64_v3_flag_sets_supported(cpu_flags: &[BTreeSet<&str>]) -> bool {
    const REQUIRED: &[&str] = &[
        // Linux exposes SSE3 as pni and LZCNT as abm in /proc/cpuinfo.
        "cx16", "lahf_lm", "popcnt", "pni", "ssse3", "sse4_1", "sse4_2", "avx", "avx2", "bmi1",
        "bmi2", "f16c", "fma", "abm", "movbe",
    ];
    !cpu_flags.is_empty()
        && cpu_flags
            .iter()
            .all(|flags| REQUIRED.iter().all(|flag| flags.contains(flag)))
}
