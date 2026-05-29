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

#[test]
fn create_scenario_default_notes_do_not_contain_todo_marker() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let home = temp_home("scenario-default-notes");

    let path = create_scenario(ScenarioCreateInput {
        name: "default-notes".to_owned(),
        force: false,
        watch_process: None,
        duration: 180,
        preset: "diagnosis".to_owned(),
        mangohud_log: None,
        notes: None,
    })
    .expect("scenario template should be created");

    let text = std::fs::read_to_string(&path).expect("scenario template should be readable");

    assert!(
        text.contains(
            "Describe the route and edit watch_process/tree_pid/pid before running this scenario."
        ),
        "generated scenario should include neutral editing guidance:\n{text}"
    );
    assert!(
        !text.contains("TODO"),
        "generated scenario should not contain TODO markers:\n{text}"
    );

    std::fs::remove_dir_all(home).ok();
}

#[test]
fn create_scenario_preserves_user_supplied_notes() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let home = temp_home("scenario-custom-notes");

    let path = create_scenario(ScenarioCreateInput {
        name: "custom-notes".to_owned(),
        force: false,
        watch_process: None,
        duration: 180,
        preset: "diagnosis".to_owned(),
        mangohud_log: None,
        notes: Some("Forest route with two camera pans.".to_owned()),
    })
    .expect("scenario template should be created");

    let scenario = load_scenario("custom-notes").expect("scenario should load");
    assert_eq!(
        scenario.notes.as_deref(),
        Some("Forest route with two camera pans.")
    );

    let text = std::fs::read_to_string(&path).expect("scenario template should be readable");
    assert!(!text.contains("TODO"));

    std::fs::remove_dir_all(home).ok();
}

fn temp_home(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "stutter-scenario-tests-{name}-{}",
        std::process::id()
    ));

    std::fs::remove_dir_all(&path).ok();
    std::fs::create_dir_all(&path).expect("create temp home");

    // SAFETY: these tests hold TEST_MUTEX while changing HOME, and restore by
    // deleting the temporary directory after each test.
    unsafe {
        std::env::set_var("HOME", &path);
    }

    path
}
