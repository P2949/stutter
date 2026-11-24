use super::*;

fn put(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn host(root: &Path) {
    let cpu = "sys/devices/system/cpu";
    put(root, &format!("{cpu}/online"), "0-11\n");
    put(
        root,
        &format!("{cpu}/cpu0/topology/thread_siblings_list"),
        "0,6\n",
    );
    put(root, "sys/devices/system/node/online", "0\n");
    let some = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
    put(root, "proc/pressure/memory", some);
    put(root, "proc/pressure/io", some);
    put(root, "proc/pressure/cpu", some);
    put(root, "proc/swaps", "Filename\tType\tSize\tUsed\tPriority\n");
    put(root, "proc/vmstat", "nr_free_pages 1\noom_kill 0\n");
    put(
        root,
        "proc/interrupts",
        "           CPU0\n  0:  1  IO-APIC  timer\n",
    );
    put(root, "proc/irq/default_smp_affinity", "fff\n");
    put(
        root,
        &format!("{cpu}/cpu0/cpufreq/scaling_governor"),
        "powersave\n",
    );
    put(
        root,
        &format!("{cpu}/cpu0/cpufreq/energy_performance_preference"),
        "balance_performance\n",
    );
    put(root, "sys/kernel/sched_ext/state", "disabled\n");
    put(
        root,
        "sys/kernel/mm/damon/admin/kdamonds/nr_kdamonds",
        "0\n",
    );
    put(root, "proc/cmdline", "BOOT_IMAGE=/vmlinuz root=/dev/sda1\n");
    put(
        root,
        "proc/cpuinfo",
        "processor\t: 0\nflags\t\t: fpu vme sse4_2\n",
    );
}

fn counts(root: &Path, name: &str) -> bool {
    readings(root)
        .into_iter()
        .find(|(counter, _)| *counter == name)
        .is_some_and(|(_, counted)| counted)
}

#[test]
fn each_fact_is_read_from_its_own_files_and_a_missing_or_odd_one_counts_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(readings(root).iter().all(|(_, counted)| !counted));
    host(root);
    assert!(readings(root).iter().all(|(name, counted)| *counted || {
        panic!("{name} was not counted on a complete host")
    }));
    // Each rule fails on its own odd shape, and only that counter goes.
    let breaks: [(&str, &str, &str); 9] = [
        (
            "single_numa_samples",
            "sys/devices/system/node/online",
            "0-1\n",
        ),
        ("memory_guard_samples", "proc/pressure/memory", "garbage\n"),
        ("memory_guard_samples", "proc/vmstat", "nr_free_pages 1\n"),
        (
            "topology_samples",
            "sys/devices/system/cpu/cpu0/topology/thread_siblings_list",
            "\n",
        ),
        ("irq_placement_samples", "proc/interrupts", "CPU0\n"),
        (
            "damon_samples",
            "sys/kernel/mm/damon/admin/kdamonds/nr_kdamonds",
            "x\n",
        ),
        (
            "resctrl_absent_samples",
            "proc/cpuinfo",
            "flags\t: fpu rdt_a\n",
        ),
        ("io_pressure_samples", "proc/pressure/cpu", "garbage\n"),
        ("boot_cmdline_samples", "proc/cmdline", "\n"),
    ];
    for (counter, file, text) in breaks {
        host(root);
        put(root, file, text);
        assert!(!counts(root, counter), "{counter} with {file}");
        let others = readings(root)
            .iter()
            .filter(|(name, counted)| *name != counter && !counted)
            .count();
        assert_eq!(others, 0, "only {counter} should go");
    }
}
