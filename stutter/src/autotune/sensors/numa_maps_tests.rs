use super::*;

fn this_process() -> (PathBuf, u32, u64) {
    let pid = std::process::id();
    let live = live_process(std::path::Path::new("/proc"), pid).unwrap();
    (PathBuf::from("/proc"), pid, live.start_ticks)
}

#[test]
fn the_target_is_read_at_most_once_per_interval_and_never_stale() {
    let (proc_root, pid, ticks) = this_process();
    let mut sensor = NumaMapsSensor::new(proc_root, pid, ticks);
    if sensor.start().is_err() {
        return; // a kernel without NUMA support: nothing to read
    }
    let first = sensor.sample(1_000);
    assert_eq!(first.scope, SensorScope::Process);
    if first.quality == ObjectiveSignalQuality::Direct {
        assert!(first.values["pages_total"] > 0.0);
    }
    let soon = sensor.sample(2_000);
    assert!(
        soon.values.is_empty(),
        "inside the interval there is no reading, not a stale one"
    );
    assert_eq!(soon.quality, ObjectiveSignalQuality::Missing);
    let later = sensor.sample(1_000 + MIN_INTERVAL_NS);
    assert_eq!(
        later.quality == ObjectiveSignalQuality::Direct,
        first.quality == ObjectiveSignalQuality::Direct
    );
}

#[test]
fn a_reused_pid_or_a_gone_process_is_never_read_as_the_target() {
    let (proc_root, pid, ticks) = this_process();
    let mut wrong_start = NumaMapsSensor::new(proc_root.clone(), pid, ticks + 1);
    assert!(matches!(wrong_start.probe(), Availability::Unavailable(_)));
    assert!(wrong_start.start().is_err());
    let reading = wrong_start.sample(5);
    assert!(reading.values.is_empty());
    assert!(matches!(wrong_start.health(), SensorHealth::Degraded(_)));

    let gone = NumaMapsSensor::new(proc_root, u32::MAX - 1, 1);
    assert!(matches!(gone.probe(), Availability::Unavailable(_)));
}

#[test]
fn a_fixture_numa_maps_gives_per_node_pages() {
    let base = std::env::temp_dir().join(format!("stutter-nm-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let dir = base.join("4242");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("numa_maps"), "7f00 default anon=10 N0=6 N1=4\n").unwrap();
    // A fake stat file whose start time field (22) is 777.
    let fields = vec!["0"; 17].join(" ");
    fs::write(
        dir.join("stat"),
        format!("4242 (game) S 1 {fields} 777 0 0\n"),
    )
    .unwrap();
    let mut sensor = NumaMapsSensor::new(base.clone(), 4242, 777);
    sensor.start().unwrap();
    let reading = sensor.sample(100);
    assert_eq!(reading.values["pages_on_node_0"], 6.0);
    assert_eq!(reading.values["pages_on_node_1"], 4.0);
    assert_eq!(reading.values["pages_total"], 10.0);
    fs::remove_dir_all(&base).unwrap();
}
