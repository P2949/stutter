//! Read-only Cyberpunk Steam-manifest preflight; installation is not entitlement or launch proof.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::Path,
};

use serde::{Deserialize, Serialize};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_GAME_EXECUTABLE: u64 = 256 * 1024 * 1024;
const MAX_TOKEN: usize = 4096;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamUpdateStatus {
    NoPendingUpdateIndicated,
    Updating,
    Unknown,
}

/// This deliberately omits account IDs, saves, launch options and other private Steam data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CyberpunkInstallation {
    pub app_id: u32,
    pub build_id: u64,
    pub installation_directory: String,
    pub manifest_update_status: SteamUpdateStatus,
    pub manifest_sha256: String,
    pub game_executable_sha256: Option<String>,
    pub entitlement_verified: bool,
    pub native_benchmark_verified: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokens(bytes: &[u8]) -> anyhow::Result<Vec<Token>> {
    anyhow::ensure!(bytes.len() <= MAX_MANIFEST as usize, "manifest too large");
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                result.push(Token::Open);
                i += 1;
            }
            b'}' => {
                result.push(Token::Close);
                i += 1;
            }
            b'"' => {
                i += 1;
                let mut text = Vec::new();
                let mut closed = false;
                while i < bytes.len() {
                    let byte = bytes[i];
                    i += 1;
                    if byte == b'"' {
                        closed = true;
                        break;
                    }
                    if byte == b'\\' {
                        let escaped = *bytes
                            .get(i)
                            .ok_or_else(|| anyhow::anyhow!("truncated escape"))?;
                        anyhow::ensure!(
                            matches!(escaped, b'\\' | b'"'),
                            "unsupported manifest escape"
                        );
                        text.push(escaped);
                        i += 1;
                    } else {
                        anyhow::ensure!(
                            byte != 0 && !byte.is_ascii_control(),
                            "invalid manifest character"
                        );
                        text.push(byte);
                    }
                    anyhow::ensure!(text.len() <= MAX_TOKEN, "manifest token too long");
                }
                anyhow::ensure!(closed, "unterminated manifest token");
                result.push(Token::Text(String::from_utf8(text)?));
            }
            _ => anyhow::bail!("unsupported manifest syntax"),
        }
        anyhow::ensure!(result.len() <= 65_536, "too many manifest tokens");
    }
    Ok(result)
}

fn parse_object(
    tokens: &[Token],
    cursor: &mut usize,
    depth: usize,
    capture: bool,
) -> anyhow::Result<BTreeMap<String, String>> {
    anyhow::ensure!(depth <= MAX_DEPTH, "manifest nesting too deep");
    let mut values = BTreeMap::new();
    let mut seen = BTreeSet::new();
    loop {
        let key = match tokens.get(*cursor) {
            Some(Token::Close) => {
                *cursor += 1;
                return Ok(values);
            }
            Some(Token::Text(key)) => {
                *cursor += 1;
                key.to_ascii_lowercase()
            }
            _ => anyhow::bail!("malformed manifest object"),
        };
        anyhow::ensure!(seen.insert(key.clone()), "duplicate manifest field");
        match tokens.get(*cursor) {
            Some(Token::Text(value)) => {
                *cursor += 1;
                if capture {
                    values.insert(key, value.clone());
                }
            }
            Some(Token::Open) => {
                *cursor += 1;
                parse_object(tokens, cursor, depth + 1, false)?;
            }
            _ => anyhow::bail!("malformed manifest value"),
        }
    }
}

fn parse_manifest(bytes: &[u8]) -> anyhow::Result<CyberpunkInstallation> {
    let tokens = tokens(bytes)?;
    anyhow::ensure!(
        matches!(tokens.first(), Some(Token::Text(name)) if name.eq_ignore_ascii_case("AppState"))
            && tokens.get(1) == Some(&Token::Open),
        "missing AppState"
    );
    let mut cursor = 2;
    let values = parse_object(&tokens, &mut cursor, 1, true)?;
    anyhow::ensure!(cursor == tokens.len(), "trailing manifest content");
    let number = |key: &str| -> anyhow::Result<u64> {
        let value = values
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("missing {key}"))?;
        anyhow::ensure!(
            !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
            "invalid {key}"
        );
        Ok(value.parse()?)
    };
    anyhow::ensure!(number("appid")? == 1_091_500, "wrong Steam AppID");
    let build_id = number("buildid")?;
    anyhow::ensure!(build_id != 0, "missing installed build");
    let installation_directory = values
        .get("installdir")
        .ok_or_else(|| anyhow::anyhow!("missing installation directory"))?;
    anyhow::ensure!(
        installation_directory == "Cyberpunk 2077",
        "unexpected installation directory"
    );
    let flags = number("stateflags")?;
    let update_status = match (
        number("targetbuildid"),
        number("bytestodownload"),
        number("bytesdownloaded"),
        number("bytestostage"),
        number("bytesstaged"),
    ) {
        (Ok(0), Ok(download), Ok(downloaded), Ok(stage), Ok(staged))
            if flags == 4 && download == downloaded && stage == staged =>
        {
            SteamUpdateStatus::NoPendingUpdateIndicated
        }
        (Ok(target), Ok(download), Ok(downloaded), Ok(stage), Ok(staged))
            if target != 0 || download != downloaded || stage != staged =>
        {
            SteamUpdateStatus::Updating
        }
        _ => SteamUpdateStatus::Unknown,
    };
    Ok(CyberpunkInstallation {
        app_id: 1_091_500,
        build_id,
        installation_directory: installation_directory.clone(),
        manifest_update_status: update_status,
        manifest_sha256: stutter_core::hash::sha256_hex(bytes),
        game_executable_sha256: None,
        entitlement_verified: false,
        native_benchmark_verified: false,
    })
}

/// Read the fixed Steam manifest and game executable without changing Steam data. This records a
/// point-in-time source identity, not entitlement, loaded-image identity, or benchmark completion.
pub fn inspect_cyberpunk_installation(
    steamapps: &Path,
    owner_uid: u32,
) -> anyhow::Result<CyberpunkInstallation> {
    anyhow::ensure!(owner_uid != 0, "Steam user must be unprivileged");
    let dir = crate::syscall::open_dir_no_symlinks(steamapps)?;
    let meta = File::from(dir.try_clone()?).metadata()?;
    anyhow::ensure!(
        meta.is_dir() && meta.uid() == owner_uid,
        "untrusted Steam library directory"
    );
    let file = crate::syscall::openat_nofollow(dir.as_fd(), "appmanifest_1091500.acf")?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == owner_uid
            && before.nlink() == 1
            && before.len() <= MAX_MANIFEST,
        "untrusted Steam manifest"
    );
    let mut bytes = Vec::new();
    (&file).take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_MANIFEST
            && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec(),
        "Steam manifest changed while reading"
    );
    let mut installation = parse_manifest(&bytes)?;
    let executable_dir = steamapps.join("common/Cyberpunk 2077/bin/x64");
    let executable_dir = crate::syscall::open_dir_no_symlinks(&executable_dir)?;
    let executable = crate::syscall::openat_nofollow(executable_dir.as_fd(), "Cyberpunk2077.exe")?;
    let exe_before = executable.metadata()?;
    anyhow::ensure!(
        exe_before.is_file()
            && exe_before.uid() == owner_uid
            && exe_before.nlink() == 1
            && exe_before.len() > 0
            && exe_before.len() <= MAX_GAME_EXECUTABLE,
        "untrusted game executable"
    );
    let mut exe_bytes = Vec::new();
    (&executable)
        .take(MAX_GAME_EXECUTABLE + 1)
        .read_to_end(&mut exe_bytes)?;
    let exe_after = executable.metadata()?;
    anyhow::ensure!(
        exe_bytes.len() as u64 == exe_before.len()
            && exe_bytes.starts_with(b"MZ")
            && exe_before.dev() == exe_after.dev()
            && exe_before.ino() == exe_after.ino()
            && exe_before.len() == exe_after.len()
            && exe_before.mtime() == exe_after.mtime()
            && exe_before.mtime_nsec() == exe_after.mtime_nsec(),
        "game executable changed or lacks an MZ signature"
    );
    let manifest_current =
        crate::syscall::openat_nofollow(dir.as_fd(), "appmanifest_1091500.acf")?.metadata()?;
    let exe_current =
        crate::syscall::openat_nofollow(executable_dir.as_fd(), "Cyberpunk2077.exe")?.metadata()?;
    let library_current =
        File::from(crate::syscall::open_dir_no_symlinks(steamapps)?).metadata()?;
    anyhow::ensure!(
        library_current.dev() == meta.dev()
            && library_current.ino() == meta.ino()
            && manifest_current.dev() == before.dev()
            && manifest_current.ino() == before.ino()
            && manifest_current.len() == before.len()
            && manifest_current.mtime() == before.mtime()
            && manifest_current.mtime_nsec() == before.mtime_nsec()
            && exe_current.dev() == exe_before.dev()
            && exe_current.ino() == exe_before.ino()
            && exe_current.len() == exe_before.len()
            && exe_current.mtime() == exe_before.mtime()
            && exe_current.mtime_nsec() == exe_before.mtime_nsec(),
        "Steam source changed during inventory"
    );
    installation.game_executable_sha256 = Some(stutter_core::hash::sha256_hex(&exe_bytes));
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(extra: &str) -> String {
        format!(
            "\"AppState\" {{ \"appid\" \"1091500\" \"buildid\" \"42\" \"installdir\" \"Cyberpunk 2077\" \"StateFlags\" \"4\" \"TargetBuildID\" \"0\" \"BytesToDownload\" \"2\" \"BytesDownloaded\" \"2\" \"BytesToStage\" \"3\" \"BytesStaged\" \"3\" \"InstalledDepots\" {{ \"1\" {{ \"appid\" \"other\" }} }} {extra} }}"
        )
    }
    #[test]
    fn manifest_extracts_only_top_level_identity_and_update_state() {
        let result = parse_manifest(manifest("").as_bytes()).unwrap();
        assert_eq!(result.build_id, 42);
        assert_eq!(
            result.manifest_update_status,
            SteamUpdateStatus::NoPendingUpdateIndicated
        );
        let updating =
            manifest("").replace("\"BytesDownloaded\" \"2\"", "\"BytesDownloaded\" \"1\"");
        assert_eq!(
            parse_manifest(updating.as_bytes())
                .unwrap()
                .manifest_update_status,
            SteamUpdateStatus::Updating
        );
        let unknown = manifest("").replace("\"TargetBuildID\" \"0\"", "");
        assert_eq!(
            parse_manifest(unknown.as_bytes())
                .unwrap()
                .manifest_update_status,
            SteamUpdateStatus::Unknown
        );
    }
    #[test]
    fn wrong_identity_duplicate_fields_and_malformed_input_are_rejected() {
        assert!(parse_manifest(manifest("\"appid\" \"1091500\"").as_bytes()).is_err());
        assert!(parse_manifest(manifest("").replace("1091500", "1").as_bytes()).is_err());
        assert!(
            parse_manifest(
                manifest("")
                    .replace("Cyberpunk 2077", "../elsewhere")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(parse_manifest(b"\"AppState\" { \"appid\" \"1091500\"").is_err());
    }

    #[test]
    fn inventory_refuses_a_symlinked_library_path() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("steamapps");
        std::fs::create_dir(&actual).unwrap();
        std::fs::write(actual.join("appmanifest_1091500.acf"), manifest("")).unwrap();
        let exe_dir = actual.join("common/Cyberpunk 2077/bin/x64");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("Cyberpunk2077.exe"), b"MZfixture").unwrap();
        assert_eq!(
            inspect_cyberpunk_installation(&actual, crate::syscall::geteuid())
                .unwrap()
                .build_id,
            42
        );
        let alias = root.path().join("alias");
        symlink(&actual, &alias).unwrap();
        assert!(inspect_cyberpunk_installation(&alias, crate::syscall::geteuid()).is_err());
        std::fs::write(exe_dir.join("Cyberpunk2077.exe"), b"not a PE").unwrap();
        assert!(inspect_cyberpunk_installation(&actual, crate::syscall::geteuid()).is_err());
    }
}
