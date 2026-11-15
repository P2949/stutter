use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::{Component, Path},
};

use super::{
    super::vdf::{VdfDocument, VdfValue},
    MAX_TOOL_FILE, ProtonCandidate, ProtonEligibility, ProtonSource, ResearchTool,
    STEAM_RUNTIME_APP_ID,
};

#[derive(Clone, Debug)]
pub(super) struct FileIdentity {
    pub(super) bytes: Vec<u8>,
    pub(super) sha256: String,
    pub(super) mode: u32,
}

pub(super) fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.uid() == right.uid()
        && left.mode() == right.mode()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

/// Whether `text` (a `uid_map`) maps uid 0 of this namespace to a real uid.
fn uid_map_maps_root(text: &str) -> bool {
    text.lines().any(|line| {
        let mut fields = line.split_whitespace().map(|field| field.parse::<u64>());
        matches!(
            (fields.next(), fields.next(), fields.next()),
            (Some(Ok(inside)), Some(Ok(_)), Some(Ok(count))) if inside == 0 && count > 0
        )
    })
}

/// Whether `actual` is the expected owner as seen from this process. Inside a container whose user
/// namespace maps only the user, root-owned files such as the Nix store are reported as the kernel's
/// overflow uid, so for an expected owner of root, and only when root is unmapped here, that uid is
/// accepted. Identity is still bound by the content digests and the frozen store path.
pub(super) fn owner_matches(actual: u32, expected: u32) -> bool {
    actual == expected
        || (expected == 0
            && actual == overflow_uid()
            && std::fs::read_to_string("/proc/self/uid_map")
                .is_ok_and(|map| !uid_map_maps_root(&map)))
}

fn overflow_uid() -> u32 {
    std::fs::read_to_string("/proc/sys/kernel/overflowuid")
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(65_534)
}

pub(super) fn read_trusted_file(
    dir: impl AsFd,
    name: &str,
    expected_uid: u32,
    max_bytes: u64,
    immutable: bool,
) -> anyhow::Result<FileIdentity> {
    let file = crate::syscall::openat_nofollow(dir.as_fd(), name)?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && owner_matches(before.uid(), expected_uid)
            && (immutable || before.nlink() == 1)
            && before.len() <= max_bytes
            && (!immutable || before.mode() & 0o222 == 0),
        "untrusted Proton file"
    );
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file).take(max_bytes + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == before.len() && same_file_identity(&before, &after),
        "Proton file changed while reading"
    );
    let current = crate::syscall::openat_nofollow(dir.as_fd(), name)?.metadata()?;
    anyhow::ensure!(
        same_file_identity(&before, &current),
        "Proton file was replaced while reading"
    );
    let sha256 = stutter_core::hash::sha256_hex(&bytes);
    Ok(FileIdentity {
        bytes,
        sha256,
        mode: before.mode(),
    })
}

pub(super) fn string<'a>(object: &'a BTreeMap<String, VdfValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(VdfValue::string)
}

pub(super) fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_steam_branch(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(super) fn validate_toolmanifests(
    toolmanifest: &[u8],
    compatibility: Option<(&[u8], &str, &str)>,
) -> anyhow::Result<()> {
    let manifest = VdfDocument::parse(toolmanifest)?;
    anyhow::ensure!(
        manifest.name == "manifest",
        "unexpected Proton toolmanifest root"
    );
    let root = manifest.root_object()?;
    anyhow::ensure!(
        string(root, "version") == Some("2")
            && string(root, "commandline") == Some("/proton %verb%")
            && string(root, "require_tool_appid") == Some(STEAM_RUNTIME_APP_ID)
            && string(root, "use_sessions") == Some("1")
            && string(root, "compatmanager_layer_name") == Some("proton"),
        "unsupported Proton Steam runtime contract"
    );
    if let Some((bytes, expected_id, expected_display_name)) = compatibility {
        let document = VdfDocument::parse(bytes)?;
        anyhow::ensure!(
            document.name == "compatibilitytools",
            "unexpected compatibility root"
        );
        let root = document.root_object()?;
        let tools = root
            .get("compat_tools")
            .and_then(VdfValue::object)
            .ok_or_else(|| anyhow::anyhow!("missing compat_tools"))?;
        anyhow::ensure!(
            tools.len() == 1,
            "compatibility manifest must register one tool"
        );
        let entry = tools
            .get(expected_id)
            .and_then(VdfValue::object)
            .ok_or_else(|| anyhow::anyhow!("compatibility tool identity mismatch"))?;
        anyhow::ensure!(
            string(entry, "install_path") == Some(".")
                && string(entry, "from_oslist") == Some("windows")
                && string(entry, "to_oslist") == Some("linux")
                && string(entry, "display_name") == Some(expected_display_name),
            "unsupported compatibility tool registration"
        );
    }
    Ok(())
}

pub(super) fn version_line(bytes: &[u8], expected: &str) -> anyhow::Result<String> {
    let value = std::str::from_utf8(bytes)?.trim();
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 512
            && !value.bytes().any(|byte| byte.is_ascii_control())
            && value.split_once(' ').is_some_and(|(timestamp, label)| {
                !timestamp.is_empty()
                    && timestamp.bytes().all(|byte| byte.is_ascii_digit())
                    && !label.is_empty()
                    && label.contains(expected)
            }),
        "Proton version file does not match pinned provenance"
    );
    Ok(value.to_owned())
}

pub(super) struct ToolSpec<'a> {
    pub(super) source: ProtonSource,
    pub(super) expected_id: &'a str,
    pub(super) expected_version: &'a str,
    pub(super) compatibility_display_name: Option<&'a str>,
    pub(super) path: &'a Path,
    pub(super) owner_uid: u32,
    pub(super) immutable: bool,
    pub(super) provenance: &'a serde_json::Value,
    pub(super) app_manifest_sha256: Option<&'a str>,
}

pub(super) fn inspect_tool_directory(spec: ToolSpec<'_>) -> anyhow::Result<ProtonCandidate> {
    let ToolSpec {
        source,
        expected_id,
        expected_version,
        compatibility_display_name,
        path,
        owner_uid,
        immutable,
        provenance,
        app_manifest_sha256,
    } = spec;
    anyhow::ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|part| !matches!(part, Component::ParentDir | Component::CurDir)),
        "invalid Proton installation path"
    );
    let dir = crate::syscall::open_dir_no_symlinks(path)?;
    let directory = File::from(dir.try_clone()?);
    let metadata = directory.metadata()?;
    anyhow::ensure!(
        metadata.is_dir()
            && owner_matches(metadata.uid(), owner_uid)
            && (!immutable || metadata.mode() & 0o222 == 0),
        "untrusted Proton installation directory"
    );

    let compatibility = if immutable {
        Some(read_trusted_file(
            dir.as_fd(),
            "compatibilitytool.vdf",
            owner_uid,
            MAX_TOOL_FILE,
            true,
        )?)
    } else {
        None
    };
    let toolmanifest = read_trusted_file(
        dir.as_fd(),
        "toolmanifest.vdf",
        owner_uid,
        MAX_TOOL_FILE,
        immutable,
    )?;
    let proton = read_trusted_file(dir.as_fd(), "proton", owner_uid, MAX_TOOL_FILE, immutable)?;
    let version_file = read_trusted_file(dir.as_fd(), "version", owner_uid, 4096, immutable)?;
    anyhow::ensure!(
        proton.bytes.starts_with(b"#!") && proton.bytes.iter().take(256).any(|byte| *byte == b'\n'),
        "Proton entrypoint is not a bounded script"
    );
    anyhow::ensure!(
        proton.mode & 0o111 != 0,
        "Proton entrypoint is not executable"
    );
    validate_toolmanifests(
        &toolmanifest.bytes,
        compatibility.as_ref().map(|file| {
            (
                file.bytes.as_slice(),
                expected_id,
                compatibility_display_name.unwrap_or_default(),
            )
        }),
    )?;
    let version = version_line(&version_file.bytes, expected_version).map_err(|error| {
        anyhow::anyhow!("{source:?} Proton version does not match pinned provenance: {error}")
    })?;
    let identity_material = serde_json::json!({
        "schema_version": 1,
        "source": source,
        "steam_tool_id": expected_id,
        "version": version,
        "provenance": provenance,
        "compatibilitytool_sha256": compatibility.as_ref().map(|file| &file.sha256),
        "toolmanifest_sha256": toolmanifest.sha256,
        "proton_sha256": proton.sha256,
        "version_file_sha256": version_file.sha256,
        "app_manifest_sha256": app_manifest_sha256,
    });
    let identity_sha256 = stutter_core::hash::sha256_hex(&serde_json::to_vec(&identity_material)?);
    Ok(ProtonCandidate {
        source,
        version: Some(version),
        tool: Some(ResearchTool {
            source,
            identity_sha256,
        }),
        eligibility: ProtonEligibility::Eligible,
        path: Some(path.to_owned()),
    })
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    #[test]
    fn root_is_unmapped_only_when_no_range_starts_at_zero() {
        assert!(uid_map_maps_root("         0          0 4294967295\n"));
        assert!(uid_map_maps_root("0 1000 1\n1000 1001 5\n"));
        assert!(!uid_map_maps_root("      1000       1000          1\n"));
        assert!(!uid_map_maps_root(""));
        assert!(!uid_map_maps_root("0 0 0\n"));
        assert!(!uid_map_maps_root("garbage\n"));
    }

    #[test]
    fn a_matching_owner_is_always_accepted_and_others_are_refused() {
        assert!(owner_matches(1000, 1000));
        assert!(owner_matches(0, 0));
        assert!(!owner_matches(1001, 1000));
        // A non-root expectation never accepts the overflow uid.
        assert!(!owner_matches(65_534, 1000));
    }
}
