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

mod cpu_power {
    use std::{fs, path::Path};

    use super::super::{Availability, CpuPowerSensor, Sensor, SensorHealth};
    use crate::autotune::objective::ObjectiveSignalQuality as Quality;

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn policy(system: &Path, number: u32, cur: u64, max: u64) {
        let dir = system.join(format!("cpu/cpufreq/policy{number}"));
        put(&dir.join("scaling_cur_freq"), &cur.to_string());
        put(&dir.join("scaling_max_freq"), &max.to_string());
        put(&dir.join("scaling_min_freq"), "800000");
        put(&dir.join("scaling_driver"), "intel_pstate");
        put(&dir.join("scaling_governor"), "powersave");
        put(
            &dir.join("energy_performance_preference"),
            "balance_performance",
        );
    }

    fn base(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("stutter-cpupower-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (root.clone(), root.join("system"), root.join("class"))
    }

    #[test]
    fn requested_versus_actual_frequency_boost_ac_and_throttle_deltas_are_read_from_sysfs() {
        let (root, system, class) = base("full");
        policy(&system, 0, 2_000_000, 4_000_000);
        policy(&system, 1, 4_000_000, 4_000_000);
        put(&system.join("cpu/intel_pstate/no_turbo"), "0");
        put(
            &system.join("cpu/cpu0/thermal_throttle/core_throttle_count"),
            "5",
        );
        put(
            &system.join("cpu/cpu0/thermal_throttle/package_throttle_count"),
            "2",
        );
        put(&class.join("power_supply/AC/type"), "Mains");
        put(&class.join("power_supply/AC/online"), "1");
        let mut sensor = CpuPowerSensor::new(&system, &class);
        assert_eq!(sensor.probe(), Availability::Available);
        sensor.start().unwrap();
        let first = sensor.sample(1);
        assert_eq!(first.quality, Quality::Direct);
        assert_eq!(first.values["policy0_cur_over_max"], 0.5);
        assert_eq!(first.values["policy1_cur_over_max"], 1.0);
        assert_eq!(
            (first.values["boost_enabled"], first.values["on_ac"]),
            (1.0, 1.0)
        );
        assert!(
            !first.values.contains_key("throttle_events_delta"),
            "no delta on the first sample"
        );
        assert!(
            first
                .provenance
                .contains("intel_pstate/powersave/balance_performance x2")
        );
        put(
            &system.join("cpu/cpu0/thermal_throttle/core_throttle_count"),
            "9",
        );
        let second = sensor.sample(2);
        assert_eq!(second.values["throttle_events_delta"], 4.0);
        // A counter that went backwards is a reset: loss, not a negative delta.
        put(
            &system.join("cpu/cpu0/thermal_throttle/core_throttle_count"),
            "1",
        );
        let reset = sensor.sample(3);
        assert_eq!(reset.lost, 1);
        assert!(!reset.values.contains_key("throttle_events_delta"));
        assert!(matches!(sensor.health(), SensorHealth::Degraded(_)));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_host_without_cpufreq_or_throttle_files_reports_less_and_invents_nothing() {
        let (root, system, class) = base("bare");
        fs::create_dir_all(&system).unwrap();
        let mut sensor = CpuPowerSensor::new(&system, &class);
        assert!(
            matches!(sensor.probe(), Availability::Unavailable(reason) if reason.contains("cpufreq"))
        );
        assert!(sensor.start().is_err());
        let empty = sensor.sample(1);
        assert_eq!(empty.quality, Quality::Missing);
        assert!(empty.values.is_empty());
        // On battery: a Mains supply that is offline, and no boost file, adds only that.
        policy(&system, 0, 1_000_000, 2_000_000);
        put(&class.join("power_supply/AC/type"), "Mains");
        put(&class.join("power_supply/AC/online"), "0");
        let battery = sensor.sample(2);
        assert_eq!(battery.values["on_ac"], 0.0);
        assert!(!battery.values.contains_key("boost_enabled"));
        fs::remove_dir_all(&root).unwrap();
    }
}

mod oom_guard {
    use std::{fs, path::Path};

    use super::super::{
        Availability, OomGuardSensor, Sensor, SensorHealth, oom_guard::session_cgroup_dir,
    };
    use crate::autotune::objective::ObjectiveSignalQuality as Quality;

    fn events(dir: &Path, oom_kill: u64, high: u64) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("memory.events"),
            format!("low 0\nhigh {high}\nmax 0\noom 0\noom_kill {oom_kill}\noom_group_kill 0\n"),
        )
        .unwrap();
    }

    #[test]
    fn the_session_slice_is_found_from_the_process_cgroup() {
        let root = Path::new("/sys/fs/cgroup");
        let text = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-x.scope\n";
        assert_eq!(
            session_cgroup_dir(root, text, 1000).unwrap(),
            root.join("user.slice/user-1000.slice")
        );
        // Not under a user slice: the process's own cgroup.
        assert_eq!(
            session_cgroup_dir(root, "0::/system.slice/foo.service\n", 1000).unwrap(),
            root.join("system.slice/foo.service")
        );
        assert!(session_cgroup_dir(root, "1:name=systemd:/x\n", 1000).is_none());
    }

    #[test]
    fn kills_and_throttles_are_reported_as_deltas_and_a_reset_is_loss() {
        let dir = std::env::temp_dir().join(format!("stutter-oom-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        events(&dir, 0, 0);
        let mut sensor = OomGuardSensor::new(dir.clone());
        assert_eq!(sensor.probe(), Availability::Available);
        sensor.start().unwrap();
        assert_eq!(
            sensor.sample(1).quality,
            Quality::Missing,
            "no delta on the first sample"
        );
        events(&dir, 2, 5);
        let second = sensor.sample(2);
        assert_eq!(
            (second.values["oom_kill_delta"], second.values["high_delta"]),
            (2.0, 5.0)
        );
        assert_eq!(second.quality, Quality::Direct);
        events(&dir, 1, 5);
        let reset = sensor.sample(3);
        assert_eq!(reset.lost, 1);
        assert!(!reset.values.contains_key("oom_kill_delta"));
        assert!(matches!(sensor.health(), SensorHealth::Degraded(_)));
        // The cgroup disappears: degraded and missing, not a crash and not zeros.
        fs::remove_dir_all(&dir).unwrap();
        let gone = sensor.sample(4);
        assert_eq!(gone.quality, Quality::Missing);
        assert!(matches!(sensor.probe(), Availability::Unavailable(_)));
    }
}

mod memory_domains {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use super::super::{Availability, DamonSensor, NumaSensor, ResctrlSensor, Sensor};
    use crate::autotune::objective::ObjectiveSignalQuality as Quality;

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("stutter-memdom-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn numastat(system: &Path, node: u32, hit: u64, local: u64, other: u64) {
        put(
            &system.join(format!("node/node{node}/numastat")),
            &format!(
                "numa_hit {hit}\nnuma_miss 0\nnuma_foreign 0\ninterleave_hit 1\nlocal_node {local}\nother_node {other}\n"
            ),
        );
    }

    #[test]
    fn numa_locality_is_a_ratio_of_the_interval_and_absent_hardware_is_unavailable() {
        let base = root("numa");
        let system = base.join("system");
        numastat(&system, 0, 100, 100, 0);
        numastat(&system, 1, 100, 100, 0);
        let mut sensor = NumaSensor::new(&system);
        assert_eq!(sensor.probe(), Availability::Available);
        sensor.start().unwrap();
        assert_eq!(sensor.sample(1).quality, Quality::Missing);
        // Node 0 gains 90 local and 10 remote allocations.
        numastat(&system, 0, 200, 190, 10);
        let second = sensor.sample(2);
        assert_eq!(second.values["local_node_delta"], 90.0);
        assert!((second.values["remote_ratio"] - 0.1).abs() < 1e-12);
        // No allocations in the next interval: no ratio rather than zero.
        let idle = sensor.sample(3);
        assert!(!idle.values.contains_key("remote_ratio"));
        // A reset is loss.
        numastat(&system, 0, 5, 5, 0);
        assert_eq!(
            sensor.sample(4).lost,
            3,
            "hit, local and other all went backwards"
        );
        let none = NumaSensor::new(&base.join("empty"));
        assert!(matches!(none.probe(), Availability::Unavailable(_)));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn damon_reports_counts_only_and_a_root_only_interface_is_permission_denied() {
        let base = root("damon");
        let admin = base.join("damon/admin/kdamonds");
        put(&admin.join("nr_kdamonds"), "2\n");
        put(&admin.join("0/state"), "on\n");
        put(&admin.join("0/contexts/nr_contexts"), "1\n");
        put(&admin.join("0/contexts/0/schemes/nr_schemes"), "3\n");
        put(&admin.join("1/state"), "off\n");
        let mut sensor = DamonSensor::new(&base);
        assert_eq!(sensor.probe(), Availability::Available);
        let reading = sensor.sample(1);
        assert_eq!(
            (
                reading.values["kdamonds"],
                reading.values["kdamonds_running"],
                reading.values["schemes_installed"]
            ),
            (2.0, 1.0, 3.0)
        );
        // Root-only: the reason is permission denied, not an empty healthy reading.
        fs::set_permissions(admin.join("nr_kdamonds"), fs::Permissions::from_mode(0o000)).unwrap();
        if crate::syscall::geteuid() != 0 {
            assert!(
                matches!(sensor.probe(), Availability::Unavailable(r) if r.contains("permission denied"))
            );
            assert_eq!(sensor.sample(2).quality, Quality::Missing);
        }
        assert!(matches!(
            DamonSensor::new(&base.join("none")).probe(),
            Availability::Unavailable(_)
        ));
        fs::set_permissions(admin.join("nr_kdamonds"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn resctrl_is_an_inventory_when_mounted_and_unavailable_when_not() {
        let base = root("resctrl");
        let mut sensor = ResctrlSensor::new(&base);
        assert!(
            matches!(sensor.probe(), Availability::Unavailable(r) if r.contains("not mounted"))
        );
        put(&base.join("resctrl/info/L3/num_closids"), "16\n");
        put(&base.join("resctrl/info/L3_MON/num_rmids"), "256\n");
        fs::create_dir_all(base.join("resctrl/mon_groups")).unwrap();
        fs::create_dir_all(base.join("resctrl/game")).unwrap();
        assert_eq!(sensor.probe(), Availability::Available);
        let values = sensor.sample(1).values;
        assert_eq!(
            (
                values["l3_closids"],
                values["l3_rmids"],
                values["allocation_groups"]
            ),
            (16.0, 256.0, 1.0)
        );
        assert!(
            !values.contains_key("mba_closids"),
            "absent hardware is absent, not zero"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}

mod escalation {
    use std::collections::BTreeMap;

    use super::super::{
        SensorReading, SensorScope,
        escalation::{Escalator, Hypothesis, Thresholds, diagnostic_sensor},
    };
    use crate::autotune::objective::ObjectiveSignalQuality;

    fn reading(sensor: &str, values: &[(&str, f64)]) -> SensorReading {
        SensorReading {
            sensor: sensor.to_owned(),
            monotonic_ns: 0,
            scope: SensorScope::System,
            quality: ObjectiveSignalQuality::Direct,
            values: values
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect::<BTreeMap<_, _>>(),
            lost: 0,
            provenance: String::new(),
        }
    }

    fn cpu(pressure: f64) -> Vec<SensorReading> {
        vec![reading("psi_system", &[("cpu_some_avg10", pressure)])]
    }

    #[test]
    fn a_hypothesis_needs_sustained_pressure_and_one_calm_sample_resets_it() {
        let mut escalator = Escalator::new(Thresholds::default());
        assert!(escalator.observe(&cpu(50.0), 1).is_none());
        assert!(escalator.observe(&cpu(50.0), 2).is_none());
        // A calm sample breaks the streak: the next two breaches are not enough.
        assert!(escalator.observe(&cpu(1.0), 3).is_none());
        assert!(escalator.observe(&cpu(50.0), 4).is_none());
        assert!(escalator.observe(&cpu(50.0), 5).is_none());
        let trigger = escalator.observe(&cpu(50.0), 6).unwrap();
        assert_eq!(trigger.hypothesis, Hypothesis::CpuContention);
        assert_eq!(trigger.diagnostic_sensor, Some("pmu_threads"));
        assert!(trigger.reason.contains("CPU pressure"));
    }

    #[test]
    fn a_trigger_cools_down_and_then_can_fire_again() {
        let thresholds = Thresholds {
            cooldown_ns: 100,
            min_consecutive: 1,
            ..Thresholds::default()
        };
        let mut escalator = Escalator::new(thresholds);
        assert!(escalator.observe(&cpu(90.0), 0).is_some());
        assert!(
            escalator.observe(&cpu(90.0), 50).is_none(),
            "inside the cooldown"
        );
        assert!(escalator.observe(&cpu(90.0), 150).is_some());
    }

    #[test]
    fn thermal_oom_and_missing_readings_map_to_the_right_hypotheses() {
        let thresholds = Thresholds {
            min_consecutive: 1,
            ..Thresholds::default()
        };
        let mut escalator = Escalator::new(thresholds);
        let hot = vec![reading("cpu_power", &[("throttle_events_delta", 4.0)])];
        let trigger = escalator.observe(&hot, 0).unwrap();
        assert_eq!(
            (trigger.hypothesis, trigger.diagnostic_sensor),
            (Hypothesis::ThermalThrottling, Some("pmu_threads"))
        );
        let oom = vec![reading("oom_guard", &[("oom_kill_delta", 1.0)])];
        let trigger = escalator.observe(&oom, 1).unwrap();
        assert_eq!(trigger.hypothesis, Hypothesis::MemoryPressure);
        assert_eq!(
            trigger.diagnostic_sensor, None,
            "memory has no expensive diagnostic yet: logged, not ignored"
        );
        assert_eq!(diagnostic_sensor(Hypothesis::IoPressure), None);
        // No readings at all, or a missing key, never triggers.
        assert!(escalator.observe(&[], 2).is_none());
        assert!(
            escalator
                .observe(&[reading("psi_system", &[])], 3)
                .is_none()
        );
    }
}
