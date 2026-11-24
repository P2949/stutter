use super::*;

fn put(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn host(root: &Path) {
    put(root, "sys/devices/system/cpu/online", "0-11\n");
    put(
        root,
        "sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
        "0,6\n",
    );
    put(root, "sys/devices/system/node/online", "0\n");
    put(
        root,
        "proc/pressure/memory",
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
    );
    put(root, "proc/swaps", "Filename\tType\tSize\tUsed\tPriority\n");
    put(root, "proc/vmstat", "nr_free_pages 1\noom_kill 0\n");
}

#[test]
fn each_fact_is_read_from_its_own_files_and_a_missing_or_odd_one_counts_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(readings(dir.path()), HostReadings::default());
    host(dir.path());
    assert_eq!(
        readings(dir.path()),
        HostReadings {
            topology: true,
            single_numa_node: true,
            memory_guard: true
        }
    );
    // Two online nodes is not the single-node host the dossier observes.
    put(dir.path(), "sys/devices/system/node/online", "0-1\n");
    assert!(!readings(dir.path()).single_numa_node);
    // A PSI file of another shape, or no OOM counter, is not the guardrail's signal.
    put(dir.path(), "proc/pressure/memory", "garbage\n");
    assert!(!readings(dir.path()).memory_guard);
    host(dir.path());
    put(dir.path(), "proc/vmstat", "nr_free_pages 1\n");
    assert!(!readings(dir.path()).memory_guard);
    host(dir.path());
    put(
        dir.path(),
        "sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
        "\n",
    );
    assert!(!readings(dir.path()).topology);
}
