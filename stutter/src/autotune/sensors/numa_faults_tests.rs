use std::fs;

use super::*;

fn proc_with(faults: u64, local: u64, migrated: u64, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-numaf-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("vmstat"),
        format!(
            "nr_free_pages 5\nnuma_hint_faults {faults}\nnuma_hint_faults_local {local}\nnuma_pages_migrated {migrated}\npgmigrate_success {migrated}\npgmigrate_fail 0\n"
        ),
    )
    .unwrap();
    dir
}

#[test]
fn hint_faults_and_migrations_are_deltas_with_a_local_ratio() {
    let dir = proc_with(100, 60, 10, "delta");
    let mut sensor = NumaFaultSensor::new(dir.clone());
    sensor.start().unwrap();
    assert!(sensor.sample(1).values.is_empty());
    let next = proc_with(200, 130, 25, "delta2");
    fs::copy(next.join("vmstat"), dir.join("vmstat")).unwrap();
    let reading = sensor.sample(2);
    assert_eq!(reading.values["numa_hint_faults_delta"], 100.0);
    assert_eq!(reading.values["numa_pages_migrated_delta"], 15.0);
    assert!((reading.values["hint_fault_local_ratio"] - 0.7).abs() < 1e-12);
}

#[test]
fn a_kernel_without_numa_balancing_is_unavailable_and_a_reset_counter_is_lost() {
    let dir = std::env::temp_dir().join(format!("stutter-numaf-none-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("vmstat"), "nr_free_pages 5\n").unwrap();
    assert!(matches!(
        NumaFaultSensor::new(dir).probe(),
        Availability::Unavailable(_)
    ));

    let dir = proc_with(100, 60, 10, "back");
    let mut sensor = NumaFaultSensor::new(dir.clone());
    sensor.start().unwrap();
    sensor.sample(1);
    let lower = proc_with(5, 5, 1, "back2");
    fs::copy(lower.join("vmstat"), dir.join("vmstat")).unwrap();
    let reading = sensor.sample(2);
    assert!(reading.lost >= 1);
    assert!(!reading.values.contains_key("numa_hint_faults_delta"));
}

#[test]
fn numa_maps_pages_are_summed_per_node_and_junk_is_skipped() {
    let text = "00400000 default file=/bin/x mapped=3 N0=3\n7f00 default anon=10 dirty=4 N0=6 N1=4\nbad line N1=oops anon=zz\n\n";
    let summary = parse_numa_maps(text);
    assert_eq!(summary.mappings, 3);
    assert_eq!(summary.pages_by_node[&0], 9);
    assert_eq!(summary.pages_by_node[&1], 4);
    assert_eq!(summary.anon_pages, 10);
    assert_eq!(summary.dirty_pages, 4);
    assert!((summary.share_on(0).unwrap() - 9.0 / 13.0).abs() < 1e-12);
    assert_eq!(parse_numa_maps("").share_on(0), None);
}

#[test]
fn this_process_numa_maps_read_for_real_does_not_panic() {
    if let Ok(text) = fs::read_to_string("/proc/self/numa_maps") {
        let summary = parse_numa_maps(&text);
        assert!(summary.mappings > 0);
    }
}
