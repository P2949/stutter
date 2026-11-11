use super::*;

const ACTIVE_SESSION: &str =
    "Id=11\nUser=1000\nActive=yes\nRemote=no\nType=wayland\nState=active\nSeat=seat0\n";

#[test]
fn accepts_only_the_exact_active_local_wayland_session() {
    assert_eq!(
        parse_session_properties(ACTIVE_SESSION, "11", 1000).unwrap(),
        "11"
    );
    assert!(parse_session_properties(ACTIVE_SESSION, "12", 1000).is_err());
    assert!(parse_session_properties(ACTIVE_SESSION, "11", 1001).is_err());
    assert!(
        parse_session_properties(
            &ACTIVE_SESSION.replace("Active=yes", "Active=no"),
            "11",
            1000
        )
        .is_err()
    );
    assert!(
        parse_session_properties(
            &ACTIVE_SESSION.replace("Remote=no", "Remote=yes"),
            "11",
            1000
        )
        .is_err()
    );
    assert!(
        parse_session_properties(
            &ACTIVE_SESSION.replace("Type=wayland", "Type=x11"),
            "11",
            1000
        )
        .is_err()
    );
}

#[test]
fn malformed_or_duplicate_logind_properties_fail_closed() {
    assert!(parse_session_properties("Id=11\nUser=1000\n", "11", 1000).is_err());
    assert!(parse_session_properties(&format!("{ACTIVE_SESSION}Id=11\n"), "11", 1000).is_err());
    assert!(!valid_session_id(""));
    assert!(!valid_session_id("11;id"));
    assert!(valid_session_id("11"));
}

#[test]
fn session_listing_selects_only_the_requested_user_and_rejects_ambiguity() {
    let listing = "11 1000 user seat0 123 user tty1 no -\n2 1000 user - 456 manager - no -\nc1 0 root - 1 user - no -\n";
    assert_eq!(session_ids(listing, 1000).unwrap(), ["11", "2"]);
    assert!(session_ids("11 1000 user\n11 1000 user\n", 1000).is_err());
    assert!(session_ids("11;id 1000 user\n", 1000).is_err());
}

#[test]
fn research_host_configuration_requires_both_absolute_inventory_paths() {
    let mut config = DaemonAutotuneConfig::default();
    assert!(configured_paths(&config).is_err());
    config.research_steamapps_dir = Some(PathBuf::from("/home/user/Steam/steamapps"));
    assert!(configured_paths(&config).is_err());
    config.research_proton_manifest = Some(PathBuf::from("relative.json"));
    assert!(configured_paths(&config).is_err());
    config.research_proton_manifest = Some(PathBuf::from("/nix/store/tool-manifest.json"));
    assert_eq!(
        configured_paths(&config).unwrap(),
        (
            PathBuf::from("/home/user/Steam/steamapps"),
            PathBuf::from("/nix/store/tool-manifest.json")
        )
    );
}
