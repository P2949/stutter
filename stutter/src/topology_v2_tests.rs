use std::{
    fs,
    path::{Path, PathBuf},
};

use super::*;

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-topo2-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("system/cpu")).unwrap();
    fs::create_dir_all(root.join("class")).unwrap();
    root
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

struct Cpu {
    id: u32,
    core: u32,
    die: Option<u32>,
    capacity: Option<u32>,
    l3_cpus: &'static str,
    l3_size: &'static str,
    policy: &'static str,
}

fn cpu(base: &Path, spec: &Cpu) {
    let dir = base.join(format!("system/cpu/cpu{}", spec.id));
    put(&dir.join("topology/core_id"), &spec.core.to_string());
    put(&dir.join("topology/physical_package_id"), "0");
    put(
        &dir.join("topology/thread_siblings_list"),
        &spec.id.to_string(),
    );
    if let Some(die) = spec.die {
        put(&dir.join("topology/die_id"), &die.to_string());
    }
    if let Some(capacity) = spec.capacity {
        put(&dir.join("cpu_capacity"), &capacity.to_string());
    }
    // L1 data and L3 unified.
    put(&dir.join("cache/index0/level"), "1");
    put(&dir.join("cache/index0/type"), "Data");
    put(
        &dir.join("cache/index0/shared_cpu_list"),
        &spec.id.to_string(),
    );
    put(&dir.join("cache/index0/size"), "32K");
    put(&dir.join("cache/index3/level"), "3");
    put(&dir.join("cache/index3/type"), "Unified");
    put(&dir.join("cache/index3/shared_cpu_list"), spec.l3_cpus);
    put(&dir.join("cache/index3/size"), spec.l3_size);
    put(&dir.join("cpufreq/related_cpus"), spec.policy);
}

fn read(base: &Path, effective: Vec<u32>) -> TopologyV2 {
    let model = TopologyModel::read_at(&base.join("system")).unwrap();
    TopologyV2::read_at(&base.join("system"), &base.join("class"), &model, effective)
}

fn online(base: &Path, list: &str) {
    put(&base.join("system/cpu/online"), list);
}

#[test]
fn a_homogeneous_single_llc_system_has_no_inferred_heterogeneity() {
    let base = root("generic");
    online(&base, "0-1");
    for id in 0..2 {
        cpu(
            &base,
            &Cpu {
                id,
                core: id,
                die: None,
                capacity: Some(1024),
                l3_cpus: "0-1",
                l3_size: "16384K",
                policy: "0 1",
            },
        );
    }
    let topo = read(&base, vec![0, 1]);
    assert_eq!(topo.llc_domains.len(), 1);
    assert_eq!(
        topo.llc_domains[0],
        LlcDomain {
            level: 3,
            cpus: vec![0, 1],
            size_kib: Some(16384)
        }
    );
    assert!(
        topo.capacity_classes.is_empty(),
        "one capacity value is not a core class"
    );
    assert!(topo.larger_cache_cpus.is_empty());
    assert!(topo.within_one_llc(&[0, 1]));
    assert_eq!(topo.frequency_domains, vec![vec![0, 1]]);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn multi_ccd_domains_with_different_verified_cache_sizes_mark_the_larger_region() {
    let base = root("ccd");
    online(&base, "0-3");
    for (id, l3, size) in [
        (0, "0-1", "32768K"),
        (1, "0-1", "32768K"),
        (2, "2-3", "98304K"),
        (3, "2-3", "98304K"),
    ] {
        cpu(
            &base,
            &Cpu {
                id,
                core: id,
                die: Some(id / 2),
                capacity: None,
                l3_cpus: l3,
                l3_size: size,
                policy: "0 1 2 3",
            },
        );
    }
    let topo = read(&base, vec![0, 1, 2, 3]);
    assert_eq!(topo.llc_domains.len(), 2);
    assert_eq!(
        topo.larger_cache_cpus,
        vec![2, 3],
        "from verified sizes, not from a part name or frequency"
    );
    assert!(
        topo.within_one_llc(&[2, 3]) && !topo.within_one_llc(&[1, 2]),
        "a split across CCDs is visible"
    );
    assert_eq!(topo.die_groups.len(), 2);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_unknown_cache_size_never_yields_a_larger_cache_region() {
    let base = root("unknown-size");
    online(&base, "0-1");
    cpu(
        &base,
        &Cpu {
            id: 0,
            core: 0,
            die: None,
            capacity: None,
            l3_cpus: "0",
            l3_size: "32768K",
            policy: "0 1",
        },
    );
    cpu(
        &base,
        &Cpu {
            id: 1,
            core: 1,
            die: None,
            capacity: None,
            l3_cpus: "1",
            l3_size: "garbage",
            policy: "0 1",
        },
    );
    assert!(read(&base, vec![0, 1]).larger_cache_cpus.is_empty());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn hybrid_capacity_classes_come_only_from_kernel_reported_capacity() {
    let base = root("hybrid");
    online(&base, "0-3");
    for (id, capacity) in [(0, 1024), (1, 1024), (2, 600), (3, 600)] {
        cpu(
            &base,
            &Cpu {
                id,
                core: id,
                die: None,
                capacity: Some(capacity),
                l3_cpus: "0-3",
                l3_size: "30720K",
                policy: "0 1 2 3",
            },
        );
    }
    let topo = read(&base, vec![0, 1, 2, 3]);
    assert_eq!(
        topo.capacity_classes,
        vec![
            CapacityClass {
                capacity: 600,
                cpus: vec![2, 3]
            },
            CapacityClass {
                capacity: 1024,
                cpus: vec![0, 1]
            },
        ]
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn numa_distance_gpu_locality_and_a_cpuset_are_read_and_missing_files_are_tolerated() {
    let base = root("numa");
    online(&base, "0");
    cpu(
        &base,
        &Cpu {
            id: 0,
            core: 0,
            die: None,
            capacity: None,
            l3_cpus: "0",
            l3_size: "8192K",
            policy: "0",
        },
    );
    put(&base.join("system/node/node0/distance"), "10 21\n");
    put(&base.join("system/node/node1/distance"), "21 10\n");
    put(&base.join("class/drm/card0/device/numa_node"), "1\n");
    put(&base.join("class/drm/card0/device/local_cpulist"), "0\n");
    put(&base.join("class/drm/card0-HDMI-A-1/status"), "connected\n");
    let topo = read(&base, vec![0]);
    assert_eq!(topo.numa_distance[&0], vec![10, 21]);
    assert_eq!(topo.numa_distance[&1], vec![21, 10]);
    assert_eq!(topo.gpu_locality.len(), 1, "connectors are not devices");
    assert_eq!(topo.gpu_locality[0].numa_node, Some(1));
    // No cache files at all: an honest empty answer, not a failure.
    let bare = root("bare");
    online(&bare, "0");
    put(&bare.join("system/cpu/cpu0/topology/core_id"), "0");
    let empty = read(&bare, vec![0]);
    assert!(
        empty.llc_domains.is_empty()
            && empty.numa_distance.is_empty()
            && empty.gpu_locality.is_empty()
    );
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(&bare).unwrap();
}

#[test]
fn any_change_in_the_topology_changes_the_fingerprint() {
    let base = root("fingerprint");
    online(&base, "0-1");
    for id in 0..2 {
        cpu(
            &base,
            &Cpu {
                id,
                core: id,
                die: None,
                capacity: None,
                l3_cpus: "0-1",
                l3_size: "16384K",
                policy: "0 1",
            },
        );
    }
    let first = read(&base, vec![0, 1]);
    assert!(first.matches(&read(&base, vec![0, 1]).fingerprint));
    // A container cpuset that narrows the usable CPUs is a different context.
    assert!(!first.matches(&read(&base, vec![0]).fingerprint));
    // A hotplugged CPU changes it too.
    online(&base, "0");
    assert!(!first.matches(&read(&base, vec![0, 1]).fingerprint));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn cache_sizes_parse_in_every_unit_and_reject_nonsense() {
    assert_eq!(parse_cache_size_kib("32K"), Some(32));
    assert_eq!(parse_cache_size_kib("96M"), Some(98304));
    assert_eq!(parse_cache_size_kib("1G"), Some(1_048_576));
    assert_eq!(parse_cache_size_kib("12"), None);
    assert_eq!(parse_cache_size_kib("xK"), None);
}
