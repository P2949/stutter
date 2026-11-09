use std::fs;

use super::*;

fn event(name: &str) -> &'static ArchEvent {
    CATALOG.iter().find(|e| e.name == name).unwrap()
}

fn fake_sysfs(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-pmu-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("uncore_imc_0/events")).unwrap();
    fs::write(root.join("uncore_imc_0/type"), "14\n").unwrap();
    fs::write(
        root.join("uncore_imc_0/events/cas_count_read"),
        "event=0x04,umask=0x03\n",
    )
    .unwrap();
    root
}

#[test]
fn vendors_are_read_from_cpuinfo_and_anything_else_is_other() {
    assert_eq!(
        vendor_from_cpuinfo("processor : 0\nvendor_id\t: GenuineIntel\n"),
        Vendor::Intel
    );
    assert_eq!(
        vendor_from_cpuinfo("vendor_id : AuthenticAMD\n"),
        Vendor::Amd
    );
    assert_eq!(
        vendor_from_cpuinfo("vendor_id : Fictional\n"),
        Vendor::Other
    );
    assert_eq!(vendor_from_cpuinfo(""), Vendor::Other);
}

#[test]
fn architecture_independent_events_resolve_to_their_kernel_abi_encoding() {
    let root = fake_sysfs("fixed");
    assert_eq!(
        resolve(event("page_faults"), Vendor::Other, &root),
        Resolved::Event { kind: 1, config: 2 }
    );
    // LL read miss: id 2, op read (0), result miss (1).
    assert_eq!(
        resolve(event("llc_read_misses"), Vendor::Other, &root),
        Resolved::Event {
            kind: 3,
            config: 2 | (1 << 16)
        }
    );
}

#[test]
fn uncore_bandwidth_events_need_the_right_vendor_and_a_real_pmu_with_a_readable_event() {
    let root = fake_sysfs("uncore");
    assert_eq!(
        resolve(event("imc_cas_reads"), Vendor::Intel, &root),
        Resolved::Event {
            kind: 14,
            config: 0x04 | (0x03 << 8)
        }
    );
    assert!(matches!(
        resolve(event("imc_cas_reads"), Vendor::Amd, &root),
        Resolved::Unavailable(reason) if reason.contains("vendor")
    ));
    assert!(matches!(
        resolve(event("df_dram_reads"), Vendor::Amd, &root),
        Resolved::Unavailable(reason) if reason.contains("not present")
    ));
    fs::write(
        root.join("uncore_imc_0/events/cas_count_read"),
        "scale=6.1\n",
    )
    .unwrap();
    assert!(matches!(
        resolve(event("imc_cas_reads"), Vendor::Intel, &root),
        Resolved::Unavailable(reason) if reason.contains("no readable event")
    ));
}

#[test]
fn resolving_everything_never_invents_an_event_for_a_missing_pmu() {
    let root = std::env::temp_dir().join("stutter-pmu-absent-root");
    let all = resolve_all("vendor_id : GenuineIntel\n", &root);
    assert_eq!(all.len(), CATALOG.len());
    for (event, resolved) in all {
        match (event.source, resolved) {
            (EventSource::Sysfs { .. }, Resolved::Event { .. }) => {
                panic!("{} resolved without a PMU", event.name)
            }
            (EventSource::Sysfs { .. }, Resolved::Unavailable(_)) => {}
            (EventSource::Fixed { .. }, Resolved::Event { .. }) => {}
            (EventSource::Fixed { .. }, Resolved::Unavailable(reason)) => {
                panic!("{}: {reason}", event.name)
            }
        }
    }
}
