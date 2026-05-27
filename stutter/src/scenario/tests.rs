use super::{model::*, *};

#[test]
fn scenario_name_rejects_path_traversal() {
    assert!(validate_scenario_name("../test").is_err());
    assert!(validate_scenario_name("test/foo").is_err());
    assert!(validate_scenario_name("test\\foo").is_err());
}

#[test]
fn scenario_name_accepts_slug() {
    assert!(validate_scenario_name("kcd-route").is_ok());
    assert!(validate_scenario_name("kcd_route_v2").is_ok());
}

#[test]
fn scenario_requires_positive_duration() {
    let mut s = ScenarioFile {
        name: "test".to_owned(),
        watch_process: Some("game".to_owned()),
        tree_pid: None,
        pid: Vec::new(),
        duration: 0,
        preset: "diagnosis".to_owned(),
        mangohud_log: None,
        expected_classes: Vec::new(),
        notes: None,
        persistent: true,
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        summary_ms: None,
        spike_us: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: None,
        cpu_freq: None,
        faults: None,
        block_io: None,
        stat_wait: None,
    };
    assert!(s.validate().is_err());
    s.duration = 1;
    assert!(s.validate().is_ok());
}

#[test]
fn scenario_requires_some_target() {
    let s = ScenarioFile {
        name: "test".to_owned(),
        watch_process: None,
        tree_pid: None,
        pid: Vec::new(),
        duration: 10,
        preset: "diagnosis".to_owned(),
        mangohud_log: None,
        expected_classes: Vec::new(),
        notes: None,
        persistent: true,
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        summary_ms: None,
        spike_us: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: None,
        cpu_freq: None,
        faults: None,
        block_io: None,
        stat_wait: None,
    };
    assert!(s.validate().is_err());
}

#[test]
fn scenario_rejects_unknown_expected_class() {
    let s = ScenarioFile {
        name: "test".to_owned(),
        watch_process: Some("game".to_owned()),
        tree_pid: None,
        pid: Vec::new(),
        duration: 10,
        preset: "diagnosis".to_owned(),
        mangohud_log: None,
        expected_classes: vec!["InvalidClass".to_owned()],
        notes: None,
        persistent: true,
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        summary_ms: None,
        spike_us: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: None,
        cpu_freq: None,
        faults: None,
        block_io: None,
        stat_wait: None,
    };
    assert!(s.validate().is_err());
}
