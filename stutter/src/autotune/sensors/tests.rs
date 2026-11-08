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
