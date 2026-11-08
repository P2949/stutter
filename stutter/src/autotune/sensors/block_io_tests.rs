use std::fs;

use super::*;

fn host(reads: u64, writes: u64, inflight: u64, tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("stutter-bio-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (proc_root, sys_root) = (base.join("proc"), base.join("sys"));
    fs::create_dir_all(&proc_root).unwrap();
    fs::create_dir_all(sys_root.join("block/sda")).unwrap();
    fs::create_dir_all(sys_root.join("block/nvme0n1")).unwrap();
    let line = |major: u32, name: &str, r: u64, w: u64, i: u64| {
        format!(
            "{major} 0 {name} {r} 0 {} 0 {w} 0 {} 0 {i} {} {}\n",
            r * 8,
            w * 8,
            r + w,
            r + w
        )
    };
    fs::write(
        proc_root.join("diskstats"),
        format!(
            "{}{}{}{}",
            line(8, "sda", reads, writes, inflight),
            line(8, "sda1", reads, writes, inflight),
            line(7, "loop0", 999, 999, 9),
            line(259, "nvme0n1", 10, 20, 0)
        ),
    )
    .unwrap();
    (proc_root, sys_root)
}

#[test]
fn only_whole_disks_count_and_deltas_and_the_inflight_gauge_are_reported() {
    let (proc_root, sys_root) = host(100, 50, 1, "a");
    let mut sensor = BlockIoSensor::new(proc_root.clone(), sys_root);
    sensor.start().unwrap();
    assert!(sensor.sample(1).values.is_empty());
    let (next, _) = host(160, 90, 3, "b");
    fs::copy(next.join("diskstats"), proc_root.join("diskstats")).unwrap();
    let reading = sensor.sample(2);
    assert_eq!(
        reading.values["io_read_ios_delta"], 60.0,
        "sda1 and loop0 are not counted"
    );
    assert_eq!(reading.values["io_write_ios_delta"], 40.0);
    assert_eq!(reading.values["io_read_sectors_delta"], 480.0);
    assert_eq!(reading.values["io_inflight"], 3.0 + 0.0);
    assert_eq!(reading.quality, ObjectiveSignalQuality::Direct);
}

#[test]
fn a_removed_disk_is_lost_not_negative_and_no_disk_is_unavailable() {
    let (proc_root, sys_root) = host(100, 50, 0, "c");
    let mut sensor = BlockIoSensor::new(proc_root.clone(), sys_root.clone());
    sensor.start().unwrap();
    sensor.sample(1);
    fs::remove_dir_all(sys_root.join("block/nvme0n1")).unwrap();
    let reading = sensor.sample(2);
    assert!(reading.lost >= 1);
    assert!(!reading.values.contains_key("io_read_ios_delta"));

    let empty = std::env::temp_dir().join(format!("stutter-bio-empty-{}", std::process::id()));
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        BlockIoSensor::new(empty.clone(), empty).probe(),
        Availability::Unavailable(_)
    ));
}

#[test]
fn the_real_host_reads_without_negative_deltas() {
    let mut sensor = BlockIoSensor::new(PathBuf::from("/proc"), PathBuf::from("/sys"));
    if sensor.start().is_err() {
        return;
    }
    sensor.sample(1);
    assert!(sensor.sample(2).values.values().all(|v| *v >= 0.0));
}
