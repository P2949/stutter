use std::{
    collections::BTreeSet,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use super::*;

const TOOLMANIFEST: &str = r#""manifest" { "version" "2" "commandline" "/proton %verb%" "require_tool_appid" "4183110" "use_sessions" "1" "compatmanager_layer_name" "proton" }"#;

#[test]
fn same_content_replacement_changes_the_hashed_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proton");
    let replacement = dir.path().join("replacement");
    std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(&replacement, b"#!/bin/sh\nexit 0\n").unwrap();
    let opened = std::fs::File::open(&path).unwrap();
    let original = opened.metadata().unwrap();
    assert!(tool::same_file_identity(
        &original,
        &opened.metadata().unwrap()
    ));
    std::fs::rename(replacement, &path).unwrap();
    assert!(!tool::same_file_identity(
        &original,
        &std::fs::metadata(path).unwrap()
    ));
}

fn compatibility(id: &str, display_name: &str) -> String {
    format!(
        "\"compatibilitytools\" {{ \"compat_tools\" {{ \"{id}\" {{ \"install_path\" \".\" \"display_name\" \"{display_name}\" \"from_oslist\" \"windows\" \"to_oslist\" \"linux\" }} }} }}"
    )
}

fn tool_dir(root: &Path, id: &str, version: &str, label: &str) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("compatibilitytool.vdf"),
        compatibility(id, &format!("{id} display")),
    )
    .unwrap();
    std::fs::hard_link(
        dir.join("compatibilitytool.vdf"),
        root.join(format!("{id}-hardlink")),
    )
    .unwrap();
    std::fs::write(dir.join("toolmanifest.vdf"), TOOLMANIFEST).unwrap();
    std::fs::write(dir.join("proton"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(dir.join("version"), format!("123 {label}-{version}\n")).unwrap();
    for name in ["compatibilitytool.vdf", "toolmanifest.vdf", "version"] {
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    std::fs::set_permissions(dir.join("proton"), std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    dir
}

fn install_steam_proton(
    steamapps: &Path,
    app_id: u32,
    name: &str,
    install_dir: &str,
    version: &str,
    state_flags: u32,
) {
    let common = steamapps.join("common");
    std::fs::create_dir_all(&common).unwrap();
    let dir = common.join(install_dir);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("toolmanifest.vdf"), TOOLMANIFEST).unwrap();
    std::fs::write(dir.join("proton"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(dir.join("version"), format!("123 proton-{version}\n")).unwrap();
    std::fs::set_permissions(dir.join("proton"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = format!(
        "\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"{name}\" \"installdir\" \"{install_dir}\" \"buildid\" \"42\" \"StateFlags\" \"{state_flags}\" \"TargetBuildID\" \"0\" \"BytesToDownload\" \"1\" \"BytesDownloaded\" \"1\" \"BytesToStage\" \"1\" \"BytesStaged\" \"1\" }}"
    );
    std::fs::write(
        steamapps.join(format!("appmanifest_{app_id}.acf")),
        manifest,
    )
    .unwrap();
}

#[test]
fn validates_exact_steam_runtime_and_custom_compatibility_identity() {
    let bytes = compatibility("proton-cachyos-slr", "proton-cachyos-slr");
    validate_toolmanifests(
        TOOLMANIFEST.as_bytes(),
        Some((bytes.as_bytes(), "proton-cachyos-slr", "proton-cachyos-slr")),
    )
    .unwrap();
    assert!(
        validate_toolmanifests(
            TOOLMANIFEST.replace("4183110", "1").as_bytes(),
            Some((bytes.as_bytes(), "proton-cachyos-slr", "proton-cachyos-slr")),
        )
        .is_err()
    );
    assert!(
        validate_toolmanifests(
            TOOLMANIFEST.as_bytes(),
            Some((bytes.as_bytes(), "another-tool", "proton-cachyos-slr")),
        )
        .is_err()
    );
}

#[test]
fn inventory_validates_four_pinned_custom_sources_and_skips_unsupported_v3() {
    let root = tempfile::tempdir().unwrap();
    let table = [
        ("cachyos", "proton-cachyos-slr", "11.0-20261015-slr"),
        ("cachyosV3", "proton-cachyos-slr-v3", "11.0-20261015-slr-v3"),
        ("ge", "ge-proton-11", "11-12"),
        ("em", "proton-em-11", "11.0-1-beta1-34-g57d3b63a"),
    ];
    let mut tools = serde_json::Map::new();
    for (key, id, version) in table {
        let path = tool_dir(root.path(), id, version, id);
        tools.insert(
            key.to_owned(),
            serde_json::json!({
                "id": id,
                "version": version,
                "displayName": format!("{id} display"),
                "path": path,
                "source": { "id": id, "version": version, "hash": format!("sha256-{id}") },
            }),
        );
    }
    let candidates = custom_candidates(
        &serde_json::to_vec(&serde_json::json!({"tools": tools})).unwrap(),
        crate::syscall::geteuid(),
        root.path(),
    )
    .unwrap();
    assert_eq!(candidates.len(), 4);
    assert!(candidates.iter().all(|candidate| candidate.tool.is_some()));
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.source)
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );
    let v3 = candidates
        .iter()
        .find(|item| item.source == ProtonSource::CachyOsV3)
        .unwrap();
    if supports_x86_64_v3() {
        assert_eq!(v3.eligibility, ProtonEligibility::Eligible);
    } else {
        assert_eq!(
            v3.eligibility,
            ProtonEligibility::Blocked(ProtonBlockReason::UnsupportedCpuIsa)
        );
    }
}

#[test]
fn serialized_inventory_never_exposes_install_paths_and_blocked_tools_are_not_runnable() {
    let path = PathBuf::from("/private/nix/store/path");
    let candidates = [ProtonSource::ValveStable, ProtonSource::ValveExperimental]
        .into_iter()
        .map(|source| ProtonCandidate {
            source,
            version: Some("test".to_owned()),
            tool: Some(ResearchTool {
                source,
                identity_sha256: "a".repeat(64),
            }),
            eligibility: ProtonEligibility::Blocked(ProtonBlockReason::Updating),
            path: Some(path.clone()),
        })
        .collect::<Vec<_>>();
    let inventory = ProtonInventory {
        schema_version: 1,
        candidates,
        steam_client: None,
    };
    assert!(inventory.tools().is_empty());
    let json = serde_json::to_string(&inventory).unwrap();
    assert!(!json.contains("/private/nix/store/path"));
    assert_eq!(
        ProtonCandidate::blocked(ProtonSource::ValveStable, ProtonBlockReason::NotInstalled)
            .eligibility,
        ProtonEligibility::Blocked(ProtonBlockReason::NotInstalled)
    );
}

#[test]
fn malformed_version_and_duplicate_tool_registration_fail_closed() {
    assert!(version_line(b"123 unrelated", "GE-Proton").is_err());
    let duplicate = r#""compatibilitytools" { "compat_tools" { "ge-proton-11" { "install_path" "." "INSTALL_PATH" "." } } }"#;
    assert!(
        validate_toolmanifests(
            TOOLMANIFEST.as_bytes(),
            Some((duplicate.as_bytes(), "ge-proton-11", "ge-proton-11"))
        )
        .is_err()
    );
}

#[test]
fn x86_64_v3_requires_every_cpu_flag_set_to_have_the_complete_abi() {
    let complete: BTreeSet<_> = [
        "cx16", "lahf_lm", "popcnt", "pni", "ssse3", "sse4_1", "sse4_2", "avx", "avx2", "bmi1",
        "bmi2", "f16c", "fma", "abm", "movbe",
    ]
    .into_iter()
    .collect();
    assert!(x86_64_v3_flag_sets_supported(std::slice::from_ref(
        &complete
    )));
    assert!(!x86_64_v3_flag_sets_supported(&[]));

    let mut incomplete = complete.clone();
    incomplete.remove("avx2");
    assert!(!x86_64_v3_flag_sets_supported(&[
        complete.clone(),
        incomplete
    ]));
}

#[test]
fn steam_webhelper_identity_requires_its_parent_and_exact_build() {
    let command = b"./steamwebhelper\0-steampid=123\0-buildid=456\0-steamid=0\0";
    assert_eq!(
        steam_webhelper_binding(command).unwrap(),
        Some((123, 456, false))
    );
    assert_eq!(
        steam_webhelper_binding(
            b"./steamwebhelper\0-steampid=123\0-buildid=456\0-steamid=987654321\0"
        )
        .unwrap(),
        Some((123, 456, true))
    );
    assert_eq!(
        steam_webhelper_binding(b"./steamwebhelper\0-buildid=456\0-steamid=0\0").unwrap(),
        None
    );
    assert_eq!(
        steam_webhelper_binding(b"other\0-steampid=123\0-buildid=456\0").unwrap(),
        None
    );
    assert!(steam_webhelper_binding(&vec![b'x'; MAX_PROC_CMDLINE + 1]).is_err());
}

#[test]
fn steam_process_identity_binds_exact_proc_start_times_and_build() {
    let root = tempfile::tempdir().unwrap();
    let steam_root = root.path().join("Steam");
    let proc_root = root.path().join("proc");
    let steamrt = steam_root.join("steamrt64");
    std::fs::create_dir_all(&steamrt).unwrap();
    std::fs::create_dir_all(&proc_root).unwrap();
    let steam_exe = steamrt.join("steam");
    let helper_exe = steamrt.join("steamwebhelper");
    std::fs::write(&steam_exe, b"steam").unwrap();
    std::fs::write(&helper_exe, b"webhelper").unwrap();

    let put_process = |pid: u32, parent: u32, ticks: u64, exe: &Path| {
        let dir = proc_root.join(pid.to_string());
        std::fs::create_dir(&dir).unwrap();
        symlink(exe, dir.join("exe")).unwrap();
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".into();
        fields[1] = parent.to_string();
        fields[19] = ticks.to_string();
        std::fs::write(
            dir.join("stat"),
            format!("{pid} (steam) {}", fields.join(" ")),
        )
        .unwrap();
    };
    put_process(100, 1, 700, &steam_exe);
    put_process(200, 1, 800, &helper_exe);
    std::fs::write(
        proc_root.join("200/cmdline"),
        b"./steamwebhelper\0-steampid=100\0-buildid=456\0-steamid=987654321\0",
    )
    .unwrap();

    let process = inspect_steam_process(&proc_root, &steam_root, crate::syscall::geteuid(), 456)
        .unwrap()
        .unwrap();
    assert_eq!(
        process,
        SteamProcessIdentity {
            steam_pid: 100,
            steam_start_ticks: 700,
            webhelper_pid: 200,
            webhelper_start_ticks: 800,
            logged_in: true,
        }
    );

    std::fs::write(
        proc_root.join("200/cmdline"),
        b"./steamwebhelper\0-steampid=100\0-buildid=455\0-steamid=987654321\0",
    )
    .unwrap();
    assert!(
        inspect_steam_process(&proc_root, &steam_root, crate::syscall::geteuid(), 456).is_err()
    );
}

#[test]
fn stable_inventory_scans_manifests_and_reports_absent_exact_updating_or_ambiguous() {
    let uid = crate::syscall::geteuid();
    let absent = tempfile::tempdir().unwrap();
    std::fs::create_dir(absent.path().join("steamapps")).unwrap();
    assert_eq!(
        inspect_valve_stable(&absent.path().join("steamapps"), uid)
            .unwrap()
            .eligibility,
        ProtonEligibility::Blocked(ProtonBlockReason::NotInstalled)
    );

    let one = tempfile::tempdir().unwrap();
    let steamapps = one.path().join("steamapps");
    std::fs::create_dir(&steamapps).unwrap();
    install_steam_proton(&steamapps, 100, "Proton 10.0", "Proton 10.0", "10.0", 4);
    let candidate = inspect_valve_stable(&steamapps, uid).unwrap();
    assert_eq!(candidate.eligibility, ProtonEligibility::Eligible);
    assert_eq!(candidate.source, ProtonSource::ValveStable);
    assert_eq!(candidate.version.as_deref(), Some("123 proton-10.0"));
    std::fs::set_permissions(
        steamapps.join("common/Proton 10.0/proton"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(inspect_valve_stable(&steamapps, uid).is_err());

    let updating = tempfile::tempdir().unwrap();
    let steamapps = updating.path().join("steamapps");
    std::fs::create_dir(&steamapps).unwrap();
    install_steam_proton(&steamapps, 101, "Proton 10.0", "Proton 10.0", "10.0", 6);
    assert_eq!(
        inspect_valve_stable(&steamapps, uid).unwrap().eligibility,
        ProtonEligibility::Blocked(ProtonBlockReason::Updating)
    );

    let multiple = tempfile::tempdir().unwrap();
    let steamapps = multiple.path().join("steamapps");
    std::fs::create_dir(&steamapps).unwrap();
    install_steam_proton(&steamapps, 102, "Proton 10.0", "Proton 10.0", "10.0", 4);
    install_steam_proton(&steamapps, 103, "Proton 9.0", "Proton 9.0", "9.0", 4);
    let candidate = inspect_valve_stable(&steamapps, uid).unwrap();
    assert_eq!(
        candidate.eligibility,
        ProtonEligibility::Blocked(ProtonBlockReason::AmbiguousStableBuild)
    );
    assert!(candidate.tool.is_some());
    assert!(candidate.path.is_none());
}

#[test]
fn stable_classification_excludes_experimental_hotfix_and_runtime_packages() {
    assert_eq!(stable_proton_version("Proton 10.0"), Some("10.0"));
    for excluded in [
        "Proton Experimental",
        "Proton Experimental Beta",
        "Proton - Hotfix",
        "Proton BattlEye Runtime",
    ] {
        assert_eq!(stable_proton_version(excluded), None);
    }
    assert_eq!(stable_proton_version("Proton 10.0 Beta"), None);
}

#[test]
fn valve_inventory_merges_registered_libraries_without_hiding_duplicates() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    install_steam_proton(&second, 100, "Proton 10.0", "Proton 10.0", "10.0", 4);
    let scan = || {
        vec![
            inspect_valve_stable(&first, uid).unwrap(),
            inspect_valve_stable(&second, uid).unwrap(),
        ]
    };
    let selected = merge_valve_candidates(ProtonSource::ValveStable, scan()).unwrap();
    assert_eq!(selected.eligibility, ProtonEligibility::Eligible);
    install_steam_proton(&first, 101, "Proton 9.0", "Proton 9.0", "9.0", 4);
    let ambiguous = merge_valve_candidates(ProtonSource::ValveStable, scan()).unwrap();
    assert_eq!(
        ambiguous.eligibility,
        ProtonEligibility::Blocked(ProtonBlockReason::AmbiguousInstallations)
    );
    assert!(ambiguous.path.is_none());
}
