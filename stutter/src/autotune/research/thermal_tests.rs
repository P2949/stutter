use super::*;

fn reading(cpu: i32, gpu: i32) -> ThermalReading {
    ThermalReading {
        cpu_mc: Some(cpu),
        gpu_mc: Some(gpu),
    }
}

fn context(start: (i32, i32), end: (i32, i32)) -> Option<ThermalContext> {
    Some(ThermalContext {
        start: reading(start.0, start.1),
        end: reading(end.0, end.1),
    })
}

fn hwmon(root: &Path, index: u32, name: &str, millidegrees: &str) {
    let dir = root.join(format!("hwmon{index}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("name"), format!("{name}\n")).unwrap();
    std::fs::write(dir.join("temp1_input"), format!("{millidegrees}\n")).unwrap();
}

#[test]
fn the_cpu_package_and_gpu_edge_are_read_from_hwmon_and_a_bad_reading_is_unknown() {
    let root = tempfile::tempdir().unwrap();
    hwmon(root.path(), 0, "nvme", "41000");
    hwmon(root.path(), 1, "coretemp", "52000");
    hwmon(root.path(), 2, "amdgpu", "47000");
    assert_eq!(read(root.path()), reading(52_000, 47_000));
    // A sensor that reports nonsense is unknown, never a temperature.
    hwmon(root.path(), 2, "amdgpu", "-5");
    assert_eq!(read(root.path()).gpu_mc, None);
    assert_eq!(
        read(&root.path().join("missing")),
        ThermalReading::default()
    );
}

#[test]
fn attempts_that_start_alike_and_end_cool_share_a_thermal_context() {
    let cool = [
        context((50_000, 45_000), (70_000, 60_000)),
        context((55_000, 47_000), (72_000, 62_000)),
    ];
    assert_eq!(state(&cool), ThermalState::Consistent);
    assert_eq!(state(&[None, None]), ThermalState::NotRecorded);
    assert_eq!(state(&[]), ThermalState::NotRecorded);
}

#[test]
fn a_hot_start_a_throttled_end_or_a_missing_record_is_not_one_comparison() {
    let hot_start = [
        context((45_000, 45_000), (70_000, 60_000)),
        context((70_000, 45_000), (72_000, 60_000)),
    ];
    assert_eq!(state(&hot_start), ThermalState::Drifted);
    let throttled = [
        context((50_000, 45_000), (70_000, 60_000)),
        context((50_000, 45_000), (96_000, 60_000)),
    ];
    assert_eq!(state(&throttled), ThermalState::Drifted);
    // Some attempts recorded one and some did not: unproven, never read as fine.
    assert_eq!(
        state(&[context((50_000, 45_000), (70_000, 60_000)), None]),
        ThermalState::Unproven
    );
    // A reading the host could not take is unknown, so the comparison is unproven.
    let blind = Some(ThermalContext {
        start: ThermalReading {
            cpu_mc: Some(50_000),
            gpu_mc: None,
        },
        end: reading(60_000, 60_000),
    });
    assert_eq!(state(&[blind, blind]), ThermalState::Unproven);
}
