use std::fs;

use super::*;

fn mount(domains: &[(&str, [&str; 3])], tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-rdt-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (name, [occupancy, total, local]) in domains {
        let dir = root.join("resctrl/mon_data").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("llc_occupancy"), format!("{occupancy}\n")).unwrap();
        fs::write(dir.join("mbm_total_bytes"), format!("{total}\n")).unwrap();
        fs::write(dir.join("mbm_local_bytes"), format!("{local}\n")).unwrap();
    }
    root
}

#[test]
fn occupancy_is_a_gauge_and_bandwidth_is_a_delta_summed_over_domains() {
    let root = mount(
        &[
            ("mon_L3_00", ["1000", "5000", "4000"]),
            ("mon_L3_01", ["500", "1000", "900"]),
        ],
        "a",
    );
    let mut sensor = ResctrlMonitorSensor::new(&root);
    sensor.start().unwrap();
    let first = sensor.sample(1);
    assert_eq!(first.values["llc_occupancy_bytes"], 1500.0);
    assert!(!first.values.contains_key("mbm_total_bytes_delta"));
    let next = mount(
        &[
            ("mon_L3_00", ["1100", "9000", "7000"]),
            ("mon_L3_01", ["600", "2000", "1500"]),
        ],
        "b",
    );
    for domain in ["mon_L3_00", "mon_L3_01"] {
        for file in ["llc_occupancy", "mbm_total_bytes", "mbm_local_bytes"] {
            let rel = format!("resctrl/mon_data/{domain}/{file}");
            fs::copy(next.join(&rel), root.join(&rel)).unwrap();
        }
    }
    let second = sensor.sample(2);
    assert_eq!(second.values["llc_occupancy_bytes"], 1700.0);
    assert_eq!(second.values["mbm_total_bytes_delta"], 5000.0);
    assert_eq!(second.values["mbm_local_bytes_delta"], 3600.0);
}

#[test]
fn an_unavailable_read_is_lost_not_zero_and_an_unmounted_resctrl_is_unavailable() {
    let root = mount(&[("mon_L3_00", ["Unavailable", "100", "50"])], "c");
    let mut sensor = ResctrlMonitorSensor::new(&root);
    sensor.start().unwrap();
    let reading = sensor.sample(1);
    assert!(!reading.values.contains_key("llc_occupancy_bytes"));
    assert_eq!(reading.lost, 1);
    assert!(matches!(sensor.health(), SensorHealth::Degraded(_)));

    let empty = std::env::temp_dir().join(format!("stutter-rdt-none-{}", std::process::id()));
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        ResctrlMonitorSensor::new(&empty).probe(),
        Availability::Unavailable(_)
    ));
    assert!(matches!(
        ResctrlMonitorSensor::new(std::path::Path::new("/sys/fs")).probe(),
        Availability::Available | Availability::Unavailable(_)
    ));
}
