use std::fs;

use super::*;

fn tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-svcmgr-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn the_manager_is_detected_from_its_runtime_directory_and_ambiguity_is_unknown() {
    let run = tree("detect");
    assert_eq!(detect(&run), ServiceManager::Unknown);
    fs::create_dir_all(run.join("systemd/system")).unwrap();
    assert_eq!(detect(&run), ServiceManager::Systemd);
    fs::create_dir_all(run.join("openrc")).unwrap();
    assert_eq!(
        detect(&run),
        ServiceManager::Unknown,
        "both present is never guessed"
    );
    fs::remove_dir_all(run.join("systemd")).unwrap();
    assert_eq!(detect(&run), ServiceManager::OpenRc);
}

#[test]
fn services_resolve_to_each_managers_cgroup_layout_only_when_the_directory_exists() {
    let cg = tree("resolve");
    fs::create_dir_all(cg.join("system.slice/stutter-x.service")).unwrap();
    fs::create_dir_all(cg.join("openrc.stutter-x")).unwrap();
    assert_eq!(
        service_cgroup(ServiceManager::Systemd, &cg, "stutter-x"),
        Some(cg.join("system.slice/stutter-x.service"))
    );
    assert_eq!(
        service_cgroup(ServiceManager::Systemd, &cg, "stutter-x.service"),
        Some(cg.join("system.slice/stutter-x.service"))
    );
    assert_eq!(
        service_cgroup(ServiceManager::OpenRc, &cg, "stutter-x"),
        Some(cg.join("openrc.stutter-x"))
    );
    assert_eq!(service_cgroup(ServiceManager::OpenRc, &cg, "absent"), None);
    assert_eq!(
        service_cgroup(ServiceManager::Unknown, &cg, "stutter-x"),
        None
    );
}

#[test]
fn names_that_could_escape_the_hierarchy_never_resolve() {
    let cg = tree("escape");
    fs::create_dir_all(cg.join("openrc.x")).unwrap();
    for bad in [
        "",
        "..",
        "../x",
        "a/b",
        ".hidden",
        "x y",
        "x\0y",
        &"a".repeat(200),
    ] {
        for manager in [ServiceManager::Systemd, ServiceManager::OpenRc] {
            assert_eq!(service_cgroup(manager, &cg, bad), None, "{bad:?}");
        }
    }
}

#[test]
fn stutters_own_services_are_recognised_by_the_same_rule_on_both_managers() {
    assert!(is_stutter_service("stutter-autotune-bounded@p2949"));
    assert!(is_stutter_service("stutter-helper"));
    for other in ["stutter-", "stutter", "sshd", "../stutter-x", "stutter x"] {
        assert!(!is_stutter_service(other), "{other}");
    }
}
