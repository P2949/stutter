use std::fs;

use super::*;

fn sys(state: &str, ops: Option<&str>, enable_seq: u64, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-scx-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let root = dir.join("kernel/sched_ext");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("state"), format!("{state}\n")).unwrap();
    fs::write(root.join("enable_seq"), format!("{enable_seq}\n")).unwrap();
    fs::write(root.join("nr_rejected"), "0\n").unwrap();
    fs::write(root.join("hotplug_seq"), "0\n").unwrap();
    if let Some(ops) = ops {
        fs::create_dir_all(root.join("root")).unwrap();
        fs::write(root.join("root/ops"), format!("{ops}\n")).unwrap();
    }
    dir
}

fn replace(from: &std::path::Path, to: &std::path::Path) {
    let _ = fs::remove_dir_all(to.join("kernel"));
    fs::create_dir_all(to.join("kernel")).unwrap();
    let status = std::process::Command::new("cp")
        .args(["-r"])
        .arg(from.join("kernel/sched_ext"))
        .arg(to.join("kernel/"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn a_disabled_kernel_reads_as_not_enabled_and_has_no_change_event() {
    let dir = sys("disabled", None, 0, "a");
    let mut sensor = SchedExtSensor::new(dir);
    sensor.start().unwrap();
    let first = sensor.sample(1);
    assert_eq!(first.values["enabled"], 0.0);
    let second = sensor.sample(2);
    assert_eq!(second.values["enable_seq_delta"], 0.0);
    assert_eq!(scheduler_change_event(&[second]), None);
}

#[test]
fn switching_the_scheduler_during_the_interval_is_a_named_safety_event() {
    let dir = sys("enabled", Some("lavd"), 1, "b");
    let mut sensor = SchedExtSensor::new(dir.clone());
    sensor.start().unwrap();
    assert_eq!(sensor.sample(1).values["enabled"], 1.0);
    let next = sys("enabled", Some("rusty"), 2, "b2");
    replace(&next, &dir);
    let reading = sensor.sample(2);
    assert_eq!(reading.values["scheduler_changed"], 1.0);
    assert_eq!(reading.values["enable_seq_delta"], 1.0);
    assert!(reading.provenance.contains("ops=rusty"));
    let event = scheduler_change_event(&[reading]).unwrap();
    assert!(event.contains("changed"));
}

#[test]
fn a_kernel_without_sched_ext_is_unavailable_and_the_real_host_reads() {
    let empty = std::env::temp_dir().join(format!("stutter-scx-none-{}", std::process::id()));
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        SchedExtSensor::new(empty).probe(),
        Availability::Unavailable(_)
    ));
    let mut real = SchedExtSensor::new(PathBuf::from("/sys"));
    if real.start().is_ok() {
        assert!(real.sample(1).values.contains_key("enabled"));
    }
}
