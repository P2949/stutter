//! Read-only Cyberpunk Steam-manifest preflight; installation is not entitlement or launch proof.
use std::{
    fs::File,
    io::Read,
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_GAME_EXECUTABLE: u64 = 256 * 1024 * 1024;
const MAX_LIBRARIES: usize = 32;

/// Discover registered Steam libraries from the client's primary SteamApps directory. Paths are
/// process-local and every directory is opened without following symlinks before it is accepted.
pub(crate) fn discover_steam_libraries(
    primary: &Path,
    owner_uid: u32,
) -> anyhow::Result<Vec<PathBuf>> {
    anyhow::ensure!(owner_uid != 0, "Steam user must be unprivileged");
    let primary_fd = crate::syscall::open_dir_no_symlinks(primary)?;
    let primary_meta = File::from(primary_fd.try_clone()?).metadata()?;
    anyhow::ensure!(
        primary_meta.is_dir() && primary_meta.uid() == owner_uid,
        "untrusted primary Steam library"
    );
    let file = crate::syscall::openat_nofollow(primary_fd.as_fd(), "libraryfolders.vdf")?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == owner_uid
            && before.nlink() == 1
            && before.len() <= MAX_MANIFEST,
        "untrusted Steam library registry"
    );
    let mut bytes = Vec::new();
    (&file).take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == before.len()
            && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec(),
        "Steam library registry changed while reading"
    );
    let document = super::vdf::VdfDocument::parse(&bytes)?;
    anyhow::ensure!(
        document.name == "libraryfolders",
        "unexpected Steam library registry root"
    );
    let root = document.root_object()?;
    anyhow::ensure!(
        !root.is_empty() && root.len() <= MAX_LIBRARIES,
        "invalid Steam library count"
    );
    let mut result = Vec::new();
    let mut identities = std::collections::BTreeSet::new();
    for (key, value) in root {
        anyhow::ensure!(
            !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid Steam library index"
        );
        let library = value
            .object()
            .ok_or_else(|| anyhow::anyhow!("invalid Steam library entry"))?;
        let path = Path::new(
            field(library, "path").ok_or_else(|| anyhow::anyhow!("Steam library has no path"))?,
        );
        anyhow::ensure!(
            path.is_absolute()
                && path
                    .components()
                    .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
            "unsafe Steam library path"
        );
        let steamapps = path.join("steamapps");
        let directory = File::from(crate::syscall::open_dir_no_symlinks(&steamapps)?);
        let meta = directory.metadata()?;
        anyhow::ensure!(
            meta.is_dir() && meta.uid() == owner_uid && identities.insert((meta.dev(), meta.ino())),
            "untrusted or duplicate Steam library"
        );
        result.push(steamapps);
    }
    anyhow::ensure!(
        identities.contains(&(primary_meta.dev(), primary_meta.ino())),
        "primary Steam library missing from registry"
    );
    let current =
        crate::syscall::openat_nofollow(primary_fd.as_fd(), "libraryfolders.vdf")?.metadata()?;
    let primary_current = File::from(crate::syscall::open_dir_no_symlinks(primary)?).metadata()?;
    anyhow::ensure!(
        current.dev() == before.dev()
            && current.ino() == before.ino()
            && current.len() == before.len()
            && current.mtime() == before.mtime()
            && current.mtime_nsec() == before.mtime_nsec()
            && primary_current.dev() == primary_meta.dev()
            && primary_current.ino() == primary_meta.ino(),
        "Steam library registry changed during discovery"
    );
    Ok(result)
}

/// Find exactly one installed Cyberpunk manifest in registered libraries; duplicate copies are
/// ambiguous and library registry app lists are not treated as ownership or installation proof.
pub(crate) fn discover_cyberpunk_installation(
    libraries: &[PathBuf],
    owner_uid: u32,
) -> anyhow::Result<(PathBuf, CyberpunkInstallation)> {
    let mut found = None;
    for steamapps in libraries {
        let dir = crate::syscall::open_dir_no_symlinks(steamapps)?;
        match crate::syscall::openat_nofollow(dir.as_fd(), "appmanifest_1091500.acf") {
            Ok(_) => {
                anyhow::ensure!(found.is_none(), "ambiguous Cyberpunk installations");
                found = Some((
                    steamapps.clone(),
                    inspect_cyberpunk_installation(steamapps, owner_uid)?,
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    found.ok_or_else(|| anyhow::anyhow!("Cyberpunk is not installed in a registered Steam library"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamUpdateStatus {
    NoPendingUpdateIndicated,
    Updating,
    Unknown,
}

fn field<'a>(
    root: &'a std::collections::BTreeMap<String, super::vdf::VdfValue>,
    name: &str,
) -> Option<&'a str> {
    root.get(name).and_then(super::vdf::VdfValue::string)
}

fn number(
    root: &std::collections::BTreeMap<String, super::vdf::VdfValue>,
    name: &str,
) -> Option<u64> {
    let value = field(root, name)?;
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())?
}

fn parse_manifest(bytes: &[u8]) -> anyhow::Result<CyberpunkInstallation> {
    let document = super::vdf::VdfDocument::parse(bytes)?;
    anyhow::ensure!(
        document.name == "appstate",
        "unexpected Steam manifest root"
    );
    let root = document.root_object()?;
    let app_id = number(root, "appid").ok_or_else(|| anyhow::anyhow!("missing Steam AppID"))?;
    let build_id = number(root, "buildid").ok_or_else(|| anyhow::anyhow!("missing build ID"))?;
    let installation_directory = field(root, "installdir")
        .ok_or_else(|| anyhow::anyhow!("missing installation directory"))?;
    anyhow::ensure!(
        app_id == 1_091_500
            && build_id != 0
            && installation_directory.len() <= 255
            && !installation_directory.contains('/')
            && Path::new(installation_directory).components().count() == 1
            && Path::new(installation_directory)
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "Steam manifest is not the expected Cyberpunk installation"
    );

    let state_flags = number(root, "stateflags");
    let target_build_id = number(root, "targetbuildid");
    let bytes_to_download = number(root, "bytestodownload");
    let bytes_downloaded = number(root, "bytesdownloaded");
    let bytes_to_stage = number(root, "bytestostage");
    let bytes_staged = number(root, "bytesstaged");
    let manifest_update_status = if target_build_id.is_some_and(|target| target != 0)
        || number(root, "stateflags").is_some_and(|flags| flags & 2 != 0)
        || matches!((bytes_to_download, bytes_downloaded), (Some(total), Some(done)) if total != done)
        || matches!((bytes_to_stage, bytes_staged), (Some(total), Some(done)) if total != done)
    {
        SteamUpdateStatus::Updating
    } else if state_flags == Some(4)
        && target_build_id == Some(0)
        && matches!((bytes_to_download, bytes_downloaded), (Some(total), Some(done)) if total == done)
        && matches!((bytes_to_stage, bytes_staged), (Some(total), Some(done)) if total == done)
    {
        SteamUpdateStatus::NoPendingUpdateIndicated
    } else {
        SteamUpdateStatus::Unknown
    };

    Ok(CyberpunkInstallation {
        app_id: app_id as u32,
        build_id,
        installation_directory: installation_directory.to_owned(),
        manifest_update_status,
        manifest_sha256: stutter_core::hash::sha256_hex(bytes),
        game_executable_sha256: None,
        launcher_executable_sha256: None,
        entitlement_verified: false,
        native_benchmark_verified: false,
    })
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_executable_sha256: Option<String>,
    pub entitlement_verified: bool,
    pub native_benchmark_verified: bool,
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
    let installation_dir = steamapps
        .join("common")
        .join(&installation.installation_directory);
    let launcher_dir = crate::syscall::open_dir_no_symlinks(&installation_dir)?;
    let launcher_meta = File::from(launcher_dir.try_clone()?).metadata()?;
    let launcher_sha256 = inspect_optional_launcher(&launcher_dir, owner_uid)?;
    let executable_dir = installation_dir.join("bin/x64");
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
    let launcher_dir_current =
        File::from(crate::syscall::open_dir_no_symlinks(&installation_dir)?).metadata()?;
    anyhow::ensure!(
        library_current.dev() == meta.dev()
            && library_current.ino() == meta.ino()
            && launcher_dir_current.dev() == launcher_meta.dev()
            && launcher_dir_current.ino() == launcher_meta.ino()
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
    installation.launcher_executable_sha256 = launcher_sha256;
    Ok(installation)
}

fn inspect_optional_launcher(dir: &impl AsFd, owner_uid: u32) -> anyhow::Result<Option<String>> {
    let file = match crate::syscall::openat_nofollow(dir.as_fd(), "REDprelauncher.exe") {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == owner_uid
            && before.nlink() == 1
            && before.len() > 0
            && before.len() <= MAX_GAME_EXECUTABLE,
        "untrusted Cyberpunk launcher"
    );
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_GAME_EXECUTABLE + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let current = crate::syscall::openat_nofollow(dir.as_fd(), "REDprelauncher.exe")?.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == before.len()
            && bytes.starts_with(b"MZ")
            && [after, current]
                .iter()
                .all(|value| value.dev() == before.dev()
                    && value.ino() == before.ino()
                    && value.len() == before.len()
                    && value.mtime() == before.mtime()
                    && value.mtime_nsec() == before.mtime_nsec()),
        "Cyberpunk launcher changed or lacks an MZ signature"
    );
    Ok(Some(stutter_core::hash::sha256_hex(&bytes)))
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
        let update_required_flag =
            manifest("").replace("\"StateFlags\" \"4\"", "\"StateFlags\" \"6\"");
        assert_eq!(
            parse_manifest(update_required_flag.as_bytes())
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
            parse_manifest(manifest("").replace("Cyberpunk 2077", "name/.").as_bytes()).is_err()
        );
        assert!(
            parse_manifest(
                manifest("")
                    .replace(
                        "\"appid\" \"1091500\"",
                        "\"appid\" { \"nested\" \"1091500\" }"
                    )
                    .as_bytes()
            )
            .is_err()
        );
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

    #[test]
    fn inventory_uses_the_safe_directory_recorded_by_the_manifest() {
        let root = tempfile::tempdir().unwrap();
        let steamapps = root.path().join("steamapps");
        let executable = steamapps.join("common/Alternate Install/bin/x64");
        std::fs::create_dir_all(&executable).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_1091500.acf"),
            manifest("").replace("Cyberpunk 2077", "Alternate Install"),
        )
        .unwrap();
        std::fs::write(executable.join("Cyberpunk2077.exe"), b"MZfixture").unwrap();
        let result = inspect_cyberpunk_installation(&steamapps, crate::syscall::geteuid()).unwrap();
        assert_eq!(result.installation_directory, "Alternate Install");
        assert!(result.game_executable_sha256.is_some());
        assert!(!result.entitlement_verified);
    }

    #[test]
    fn inventory_attests_the_launcher_when_present_and_rejects_substitution() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let steamapps = root.path().join("steamapps");
        let game = steamapps.join("common/Cyberpunk 2077");
        std::fs::create_dir_all(game.join("bin/x64")).unwrap();
        std::fs::write(steamapps.join("appmanifest_1091500.acf"), manifest("")).unwrap();
        std::fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"MZgame").unwrap();
        let launcher = game.join("REDprelauncher.exe");
        let uid = crate::syscall::geteuid();
        assert!(
            inspect_cyberpunk_installation(&steamapps, uid)
                .unwrap()
                .launcher_executable_sha256
                .is_none()
        );
        std::fs::write(&launcher, b"MZlauncher").unwrap();
        assert_eq!(
            inspect_cyberpunk_installation(&steamapps, uid)
                .unwrap()
                .launcher_executable_sha256,
            Some(stutter_core::hash::sha256_hex(b"MZlauncher"))
        );
        std::fs::write(&launcher, b"invalid").unwrap();
        assert!(inspect_cyberpunk_installation(&steamapps, uid).is_err());
        std::fs::remove_file(&launcher).unwrap();
        symlink(game.join("bin/x64/Cyberpunk2077.exe"), &launcher).unwrap();
        assert!(inspect_cyberpunk_installation(&steamapps, uid).is_err());
    }

    #[test]
    fn registered_libraries_find_a_secondary_install_and_reject_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("primary");
        let secondary = root.path().join("secondary");
        std::fs::create_dir_all(primary.join("steamapps")).unwrap();
        let game = secondary.join("steamapps/common/Installed Game/bin/x64");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Cyberpunk2077.exe"), b"MZfixture").unwrap();
        let game_manifest = manifest("").replace("Cyberpunk 2077", "Installed Game");
        std::fs::write(
            secondary.join("steamapps/appmanifest_1091500.acf"),
            &game_manifest,
        )
        .unwrap();
        let registry = |second: &Path| {
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
                primary.display(),
                second.display()
            )
        };
        let registry_path = primary.join("steamapps/libraryfolders.vdf");
        std::fs::write(&registry_path, registry(&secondary)).unwrap();
        let uid = crate::syscall::geteuid();
        let libraries = discover_steam_libraries(&primary.join("steamapps"), uid).unwrap();
        let (library, game) = discover_cyberpunk_installation(&libraries, uid).unwrap();
        assert_eq!(library, secondary.join("steamapps"));
        assert_eq!(game.installation_directory, "Installed Game");
        assert!(!game.entitlement_verified);
        std::fs::create_dir_all(primary.join("steamapps/common/Installed Game/bin/x64")).unwrap();
        std::fs::write(
            primary.join("steamapps/common/Installed Game/bin/x64/Cyberpunk2077.exe"),
            b"MZfixture",
        )
        .unwrap();
        std::fs::write(
            primary.join("steamapps/appmanifest_1091500.acf"),
            &game_manifest,
        )
        .unwrap();
        assert!(discover_cyberpunk_installation(&libraries, uid).is_err());
        std::fs::write(&registry_path, registry(&primary)).unwrap();
        assert!(discover_steam_libraries(&primary.join("steamapps"), uid).is_err());
    }

    #[test]
    fn registered_library_symlinks_and_unlisted_primary_fail_closed() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("primary");
        let secondary = root.path().join("secondary");
        std::fs::create_dir_all(primary.join("steamapps")).unwrap();
        std::fs::create_dir_all(secondary.join("steamapps")).unwrap();
        let alias = root.path().join("alias");
        symlink(&secondary, &alias).unwrap();
        let registry = primary.join("steamapps/libraryfolders.vdf");
        let value = |path: &Path| {
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
                path.display()
            )
        };
        std::fs::write(&registry, value(&alias)).unwrap();
        assert!(
            discover_steam_libraries(&primary.join("steamapps"), crate::syscall::geteuid())
                .is_err()
        );
        std::fs::write(&registry, value(&secondary)).unwrap();
        assert!(
            discover_steam_libraries(&primary.join("steamapps"), crate::syscall::geteuid())
                .is_err()
        );
    }
}
