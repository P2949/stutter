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

mod guard_event {
    use std::collections::BTreeMap;

    use super::super::{SensorReading, SensorScope, memory_guard_event};
    use crate::autotune::objective::ObjectiveSignalQuality;

    fn guard(values: &[(&str, f64)]) -> SensorReading {
        SensorReading {
            sensor: "oom_guard".into(),
            monotonic_ns: 0,
            scope: SensorScope::Cgroup,
            quality: ObjectiveSignalQuality::Direct,
            values: values
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect::<BTreeMap<_, _>>(),
            lost: 0,
            provenance: String::new(),
        }
    }

    #[test]
    fn a_kill_oom_max_or_high_event_is_a_guard_event_and_quiet_or_missing_readings_are_not() {
        for key in ["oom_kill_delta", "oom_delta", "max_delta", "high_delta"] {
            let event = memory_guard_event(&[guard(&[(key, 2.0)])]).unwrap();
            assert!(event.contains(key), "{event}");
        }
        assert_eq!(
            memory_guard_event(&[guard(&[("oom_kill_delta", 0.0), ("high_delta", 0.0)])]),
            None
        );
        assert_eq!(memory_guard_event(&[]), None);
        assert_eq!(memory_guard_event(&[guard(&[])]), None);
        let other_sensor = SensorReading {
            sensor: "psi_system".into(),
            ..guard(&[("oom_kill_delta", 9.0)])
        };
        assert_eq!(
            memory_guard_event(&[other_sensor]),
            None,
            "only the guard sensor counts"
        );
    }
}

mod thermal_guard {
    use std::collections::BTreeMap;

    use super::super::{SensorReading, SensorScope, thermal_guard_event};
    use crate::autotune::objective::ObjectiveSignalQuality;

    fn power(events: Option<f64>) -> SensorReading {
        SensorReading {
            sensor: "cpu_power".into(),
            monotonic_ns: 0,
            scope: SensorScope::System,
            quality: ObjectiveSignalQuality::Direct,
            values: events
                .map(|e| ("throttle_events_delta".to_owned(), e))
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            lost: 0,
            provenance: String::new(),
        }
    }

    #[test]
    fn throttling_in_the_interval_is_a_guard_event_and_its_absence_is_not() {
        assert!(
            thermal_guard_event(&[power(Some(3.0))])
                .unwrap()
                .contains("3 thermal")
        );
        assert_eq!(thermal_guard_event(&[power(Some(0.0))]), None);
        assert_eq!(
            thermal_guard_event(&[power(None)]),
            None,
            "no counter is no evidence either way"
        );
        assert_eq!(thermal_guard_event(&[]), None);
    }
}
