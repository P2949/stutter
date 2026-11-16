//! What a game process actually has loaded, read from its own memory map. The selected Proton label
//! says what Stutter asked for; this records what ran: whether Wine, VKD3D-Proton and DXVK files were
//! mapped, from which Proton build roots, and a digest of the mapped runtime file set. It is a
//! receipt for comparison across arms, not an identity proof: container path layouts are not yet
//! observed, so nothing is rejected on it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

const MAX_ROOTS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadedRuntime {
    pub mapped_file_count: u32,
    pub wine_loaded: bool,
    pub vkd3d_loaded: bool,
    pub dxvk_loaded: bool,
    /// Proton build roots (the path up to and including `/files`) that supplied mapped files.
    pub proton_roots: Vec<String>,
    /// Digest of the sorted, de-duplicated mapped runtime file paths.
    pub runtime_paths_sha256: String,
}

/// Summarize `/proc/<pid>/maps`: the file-backed mappings that belong to the Wine/Proton runtime.
pub fn summarize(maps: &[u8]) -> anyhow::Result<LoadedRuntime> {
    let text = std::str::from_utf8(maps)?;
    let mut files = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let (mut wine, mut vkd3d, mut dxvk) = (false, false, false);
    for line in text.lines() {
        // address perms offset dev inode [path]; the path may contain spaces.
        let Some(path) = line.splitn(6, ' ').nth(5).map(str::trim) else {
            continue;
        };
        if !path.starts_with('/') {
            continue;
        }
        let lower = path.to_ascii_lowercase();
        let is_vkd3d = lower.contains("vkd3d");
        let is_dxvk = lower.contains("dxvk") || lower.contains("/dxvk/");
        let is_wine =
            lower.contains("/wine/") || lower.contains("/wine64") || lower.contains("ntdll");
        if !(is_vkd3d || is_dxvk || is_wine) {
            continue;
        }
        wine |= is_wine;
        vkd3d |= is_vkd3d;
        dxvk |= is_dxvk;
        if let Some(index) = path.find("/files/") {
            roots.insert(path[..index + "/files".len()].to_owned());
        }
        files.insert(path.to_owned());
    }
    anyhow::ensure!(
        !files.is_empty(),
        "no Wine or Proton runtime files are mapped"
    );
    let joined = files.iter().cloned().collect::<Vec<_>>().join("\n");
    Ok(LoadedRuntime {
        mapped_file_count: files.len().min(u32::MAX as usize) as u32,
        wine_loaded: wine,
        vkd3d_loaded: vkd3d,
        dxvk_loaded: dxvk,
        proton_roots: roots.into_iter().take(MAX_ROOTS).collect(),
        runtime_paths_sha256: stutter_core::hash::sha256_hex(joined.as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPS: &str = "\
7f0000000000-7f0000100000 r--p 00000000 00:20 11 /nix/store/aaa-proton-ge/files/lib/wine/x86_64-unix/ntdll.so
7f0000100000-7f0000200000 r-xp 00000000 00:20 12 /nix/store/aaa-proton-ge/files/lib/wine/vkd3d-proton/d3d12.dll
7f0000200000-7f0000300000 r-xp 00000000 00:20 13 /nix/store/aaa-proton-ge/files/lib/wine/dxvk/dxgi.dll
7f0000300000-7f0000400000 r--p 00000000 00:20 14 /usr/lib/libc.so.6
7f0000400000-7f0000500000 rw-p 00000000 00:00 0 
7f0000500000-7f0000600000 r--p 00000000 00:00 0 [heap]
";

    #[test]
    fn the_wine_runtime_files_that_are_mapped_are_summarized_and_others_ignored() {
        let runtime = summarize(MAPS.as_bytes()).unwrap();
        assert_eq!(runtime.mapped_file_count, 3);
        assert!(runtime.wine_loaded && runtime.vkd3d_loaded && runtime.dxvk_loaded);
        assert_eq!(
            runtime.proton_roots,
            vec!["/nix/store/aaa-proton-ge/files".to_owned()]
        );
        assert_eq!(runtime.runtime_paths_sha256.len(), 64);
    }

    #[test]
    fn the_digest_follows_the_runtime_file_set_and_an_unmapped_runtime_is_an_error() {
        let same = summarize(MAPS.as_bytes()).unwrap();
        let other =
            summarize(MAPS.replace("aaa-proton-ge", "bbb-proton-cachy").as_bytes()).unwrap();
        assert_ne!(same.runtime_paths_sha256, other.runtime_paths_sha256);
        assert!(summarize(b"7f00-7f01 r--p 0 00:20 1 /usr/lib/libc.so.6\n").is_err());
    }
}
