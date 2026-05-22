use super::super::*;

#[test]
fn profile_offline_cpu_warnings_detects_rule_with_offline_cpus() {
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-3").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };
    let online = CpuMask::parse("0-1").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].rule_index, 0);
    assert_eq!(warnings[0].requested, "0-3");
    assert_eq!(warnings[0].online, "0-1");
}

#[test]
fn profile_offline_cpu_warnings_empty_when_subset() {
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-1").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };
    let online = CpuMask::parse("0-3").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert!(warnings.is_empty());
}

#[test]
fn profile_offline_cpu_warnings_multiple_rules_report_correct_indexes() {
    let profile = Profile {
        name: "test".to_owned(),
        rules: vec![
            ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            },
            ProfileRule {
                affinity: Some(CpuMask::parse("2-3").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::GameHelper],
                match_comm: Vec::new(),
            },
        ],
    };
    let online = CpuMask::parse("0-1").unwrap();

    let warnings = profile_offline_cpu_warnings(&profile, &online);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].rule_index, 1);
    assert_eq!(warnings[0].requested, "2-3");
    assert_eq!(warnings[0].online, "0-1");
}

#[test]
fn profile_rule_overlap_warnings_broad_game_before_specific_render_thread_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "0-7"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "2-5"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}

#[test]
fn profile_rule_overlap_warnings_disjoint_classes_do_not_warn() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "0-7"

        [[profile.rules]]
        match_class = ["Compositor"]
        affinity = "8-11"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert!(warnings.is_empty());
}

#[test]
fn profile_rule_overlap_warnings_catch_all_before_anything_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        affinity = "0-7"

        [[profile.rules]]
        match_class = ["Game"]
        affinity = "2-5"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}

#[test]
fn profile_rule_overlap_warnings_exact_same_comm_warns() {
    let profile = parse_profiles(
        r#"
        [[profile]]
        name = "test"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "0-3"

        [[profile.rules]]
        match_comm = ["RenderThread"]
        affinity = "4-7"
        "#,
    )
    .unwrap()
    .pop()
    .unwrap();

    let warnings = profile_rule_overlap_warnings(&profile.rules);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].earlier_rule, 0);
    assert_eq!(warnings[0].later_rule, 1);
}
