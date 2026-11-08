use std::fs;

use super::*;

fn host(c1: u64, c6: u64, energy: Option<u64>, tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("stutter-idle-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (system, class) = (base.join("sys"), base.join("class"));
    for cpu in 0..2 {
        for (state, name, time) in [(0, "C1", c1), (1, "C6", c6)] {
            let dir = system.join(format!("cpu/cpu{cpu}/cpuidle/state{state}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            fs::write(dir.join("time"), format!("{time}\n")).unwrap();
        }
    }
    fs::create_dir_all(system.join("cpu/cpufreq")).unwrap();
    if let Some(energy) = energy {
        for zone in ["intel-rapl:0", "intel-rapl:0:0"] {
            let dir = class.join("powercap").join(zone);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("energy_uj"), format!("{energy}\n")).unwrap();
        }
    }
    (system, class)
}

#[test]
fn residency_sums_over_cpus_and_only_the_package_zone_counts_for_energy() {
    let (system, class) = host(100, 1000, Some(5_000), "a");
    let mut sensor = CpuIdleSensor::new(&system, &class);
    sensor.start().unwrap();
    assert!(sensor.sample(1).values.is_empty());
    let (next, next_class) = host(150, 1900, Some(8_000), "b");
    for (from, to) in [(&next, &system), (&next_class, &class)] {
        fs::remove_dir_all(to).unwrap();
        let status = std::process::Command::new("cp")
            .arg("-r")
            .arg(from)
            .arg(to)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let reading = sensor.sample(2);
    assert_eq!(
        reading.values["idle_us_c1_delta"], 100.0,
        "two CPUs, 50 each"
    );
    assert_eq!(reading.values["idle_us_c6_delta"], 1800.0);
    assert_eq!(
        reading.values["package_energy_uj_delta"], 3000.0,
        "the sub-zone is not added to the package"
    );
}

#[test]
fn unreadable_energy_is_absent_not_zero_and_a_backwards_counter_is_lost() {
    let (system, class) = host(100, 1000, None, "c");
    let mut sensor = CpuIdleSensor::new(&system, &class);
    sensor.start().unwrap();
    sensor.sample(1);
    let reading = sensor.sample(2);
    assert!(!reading.values.contains_key("package_energy_uj_delta"));
    assert_eq!(reading.values["idle_us_c1_delta"], 0.0);

    let dir = system.join("cpu/cpu0/cpuidle/state0");
    fs::write(dir.join("time"), "1\n").unwrap();
    let lower = sensor.sample(3);
    assert!(lower.lost >= 1);
    assert!(!lower.values.contains_key("idle_us_c1_delta"));
}

#[test]
fn a_host_without_cpuidle_is_unavailable_and_the_real_host_reads() {
    let empty = std::env::temp_dir().join(format!("stutter-idle-none-{}", std::process::id()));
    fs::create_dir_all(empty.join("cpu")).unwrap();
    assert!(matches!(
        CpuIdleSensor::new(&empty, &empty).probe(),
        Availability::Unavailable(_)
    ));
    let mut real = CpuIdleSensor::new(
        std::path::Path::new("/sys/devices/system"),
        std::path::Path::new("/sys/class"),
    );
    if real.start().is_ok() {
        real.sample(1);
        assert!(real.sample(2).values.values().all(|v| *v >= 0.0));
    }
}
