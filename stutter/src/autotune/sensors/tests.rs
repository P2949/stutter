use std::fs;

use super::*;
use crate::autotune::objective::ObjectiveSignalQuality;

fn proc_with(cpu_total: u64, mem_total: u64) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-psi-{}-{cpu_total}", std::process::id()));
    fs::create_dir_all(root.join("pressure")).unwrap();
    write(&root, cpu_total, mem_total);
    root
}

fn write(root: &std::path::Path, cpu_total: u64, mem_total: u64) {
    fs::write(
        root.join("pressure/cpu"),
        format!("some avg10=1.50 avg60=1.00 avg300=0.50 total={cpu_total}\n"),
    )
    .unwrap();
    fs::write(
        root.join("pressure/memory"),
        format!("some avg10=0.10 avg60=0.00 avg300=0.00 total={mem_total}\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n"),
    )
    .unwrap();
    fs::write(root.join("pressure/io"), "some avg10=0.00 avg60=0.00 avg300=0.00 total=5\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=5\n").unwrap();
}

#[test]
fn psi_reports_deltas_after_the_first_sample_and_flags_a_reset() {
    let root = proc_with(1000, 200);
    let mut sensor = PsiSensor::system(&root);
    assert_eq!(sensor.probe(), Availability::Available);
    sensor.start().unwrap();
    let first = sensor.sample(1);
    assert_eq!(
        first.quality,
        ObjectiveSignalQuality::Missing,
        "no delta yet"
    );
    assert_eq!(first.values["cpu_some_avg10"], 1.5);
    write(&root, 1600, 260);
    let second = sensor.sample(2);
    assert_eq!(second.quality, ObjectiveSignalQuality::Direct);
    assert_eq!(second.values["cpu_some_stall_us"], 600.0);
    assert_eq!(second.values["memory_some_stall_us"], 60.0);
    assert_eq!(second.lost, 0);
    assert_eq!(sensor.health(), SensorHealth::Healthy);
    write(&root, 10, 260);
    let reset = sensor.sample(3);
    assert_eq!(reset.lost, 1);
    assert!(!reset.values.contains_key("cpu_some_stall_us"));
    assert!(matches!(sensor.health(), SensorHealth::Degraded(_)));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_feature_is_unavailable_with_its_real_reason() {
    let sensor = PsiSensor::system(std::path::Path::new("/nonexistent-proc"));
    assert!(
        matches!(sensor.probe(), Availability::Unavailable(reason) if reason.contains("nonexistent-proc"))
    );
    let mut registry = SensorRegistry::default();
    registry.register(Box::new(sensor));
    assert!(matches!(
        registry.activate("psi_system", 0),
        Err(AdmissionDenied::Unavailable(_))
    ));
    assert_eq!(registry.activate("nope", 0), Err(AdmissionDenied::Unknown));
}

struct Fake {
    id: &'static str,
    cost: SensorCost,
}

impl Sensor for Fake {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: self.id,
            cost: self.cost,
            scope: SensorScope::System,
            requires: "nothing",
        }
    }
    fn probe(&self) -> Availability {
        Availability::Available
    }
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        SensorReading {
            sensor: self.id.to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: ObjectiveSignalQuality::Direct,
            values: Default::default(),
            lost: 0,
            provenance: "fake".into(),
        }
    }
    fn health(&self) -> SensorHealth {
        SensorHealth::Healthy
    }
    fn stop(&mut self) {}
}

#[test]
fn expensive_sensors_run_one_at_a_time_with_a_cap_and_a_cooling_period() {
    let budget = ExpensiveBudget {
        max_concurrent: 1,
        max_duration_ns: 100,
        cooling_ns: 500,
    };
    let mut registry = SensorRegistry::new(budget);
    for (id, cost) in [
        ("cheap", SensorCost::Cheap),
        ("perf", SensorCost::Escalation),
        ("bpf", SensorCost::Moderate),
        ("vtune", SensorCost::Offline),
    ] {
        registry.register(Box::new(Fake { id, cost }));
    }
    registry.activate("cheap", 0).unwrap();
    registry.activate("perf", 0).unwrap();
    assert_eq!(
        registry.activate("bpf", 0),
        Err(AdmissionDenied::ConcurrencyLimit)
    );
    assert_eq!(registry.activate("vtune", 0), Err(AdmissionDenied::Offline));
    // Within the cap both cheap and expensive are sampled.
    assert_eq!(registry.sample_all(50).len(), 2);
    // Past the cap the expensive one is stopped; the cheap one continues.
    let readings = registry.sample_all(200);
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].sensor, "cheap");
    // Cooling blocks the next expensive sensor, then it is admitted.
    assert_eq!(
        registry.activate("bpf", 300),
        Err(AdmissionDenied::Cooling { remaining_ns: 400 })
    );
    registry.activate("bpf", 800).unwrap();
    assert!(registry.overhead_ns().contains_key("perf"));
}

mod pmu {
    use std::collections::BTreeMap;

    use stutter_core::ids::Tid;

    use super::super::{PmuSensor, Sensor, pmu::reading_from_deltas};
    use crate::{
        autotune::objective::ObjectiveSignalQuality as Quality,
        perf_counters::{CpuPerfConfig, CpuPerfDelta},
    };

    fn delta(cycles: u64, instructions: u64, running: u64, enabled: u64) -> CpuPerfDelta {
        CpuPerfDelta {
            cycles: Some(cycles),
            instructions: Some(instructions),
            cache_references: Some(1000),
            cache_misses: Some(50),
            time_enabled_ns: Some(enabled),
            time_running_ns: Some(running),
            multiplexed: running != enabled,
            ..CpuPerfDelta::default()
        }
    }

    fn tid(value: u32) -> Tid {
        Tid::from(value)
    }

    #[test]
    fn a_fully_counting_set_of_threads_gives_a_direct_reading_with_derived_ratios() {
        let mut deltas = BTreeMap::new();
        deltas.insert(tid(1), delta(1000, 2000, 100, 100));
        deltas.insert(tid(2), delta(1000, 2000, 100, 100));
        let reading = reading_from_deltas(&deltas, 7);
        assert_eq!(reading.quality, Quality::Direct);
        assert_eq!(reading.values["ipc"], 2.0);
        assert_eq!(reading.values["cache_mpki"], 25.0);
        assert_eq!(reading.values["cache_miss_rate"], 0.05);
        assert_eq!(reading.values["running_ratio"], 1.0);
        assert_eq!(reading.lost, 0);
    }

    #[test]
    fn multiplexing_a_low_running_share_and_unusable_threads_never_pose_as_direct() {
        let mut multiplexed = BTreeMap::new();
        multiplexed.insert(tid(1), delta(1000, 2000, 90, 100));
        assert_eq!(
            reading_from_deltas(&multiplexed, 0).quality,
            Quality::Approximate
        );

        let mut starved = BTreeMap::new();
        starved.insert(tid(1), delta(1000, 2000, 20, 100));
        assert_eq!(
            reading_from_deltas(&starved, 0).quality,
            Quality::Approximate
        );

        let mut broken = BTreeMap::new();
        broken.insert(
            tid(1),
            CpuPerfDelta {
                unavailable_reason: Some("denied".into()),
                ..CpuPerfDelta::default()
            },
        );
        broken.insert(tid(2), delta(1000, 2000, 100, 100));
        let mixed = reading_from_deltas(&broken, 0);
        assert_eq!((mixed.lost, mixed.values["threads"]), (1, 1.0));

        let none = reading_from_deltas(&BTreeMap::new(), 0);
        assert_eq!(none.quality, Quality::Missing);
        assert!(!none.values.contains_key("ipc"));
    }

    #[test]
    fn a_zero_denominator_gives_no_ratio() {
        let mut deltas = BTreeMap::new();
        deltas.insert(tid(1), delta(0, 0, 100, 100));
        let reading = reading_from_deltas(&deltas, 0);
        assert!(!reading.values.contains_key("ipc"));
        assert!(!reading.values.contains_key("cache_mpki"));
        assert_eq!(reading.quality, Quality::Missing);
    }

    #[test]
    fn the_sensor_probe_reports_availability_honestly_and_a_targetless_sample_is_missing() {
        let mut sensor = PmuSensor::new(CpuPerfConfig {
            include_kernel: false,
            max_tasks: 4,
            collect_cache_refs: true,
        });
        match sensor.start() {
            Ok(()) => {
                // Started but with no target chosen: nothing is invented.
                assert_eq!(sensor.sample(1).quality, Quality::Missing);
                sensor.stop();
            }
            Err(reason) => assert!(reason.contains("perf_event_open"), "{reason}"),
        }
    }
}
