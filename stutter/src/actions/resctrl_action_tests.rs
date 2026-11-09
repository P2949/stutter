use std::fs;

use super::*;

fn mount(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("stutter-resctrl-act-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("info/L3")).unwrap();
    fs::write(root.join("info/L3/cbm_mask"), "fff\n").unwrap();
    fs::write(root.join("info/L3/min_cbm_bits"), "2\n").unwrap();
    fs::write(root.join("info/L3/num_closids"), "8\n").unwrap();
    fs::write(root.join("schemata"), "L3:0=fff;1=fff\n").unwrap();
    root
}

fn action(root: PathBuf) -> ResctrlCatAction {
    ResctrlCatAction {
        root,
        request: CatRequest {
            name: "t1".into(),
            mask: 0xf0,
            ..Default::default()
        },
    }
}

#[test]
fn the_default_policy_denies_and_the_action_is_high_risk_system_wide() {
    let action = action(mount("deny"));
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(action.apply().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    assert!(action.descriptor().touches_system_wide_state);
    assert!(!action.root.join("stutter-cat-t1").exists());
}

#[test]
fn an_allowed_apply_creates_the_group_and_rollback_removes_exactly_it() {
    let root = mount("cycle");
    let action = action(root.clone());
    let policy = ResctrlActionPolicy { allow: true };
    let token = action.apply_with_policy(&policy).unwrap();
    assert_eq!(token.kind(), "resctrl-group-remove");
    assert!(action.verify().unwrap().applied);
    let json = serde_json::to_string(&token).unwrap();
    let back: RollbackToken = serde_json::from_str(&json).unwrap();
    assert_eq!(back, token);
    action.rollback(&token).unwrap();
    assert!(!root.join("stutter-cat-t1").exists());
    // A second rollback of the same token finds the group already gone.
    action.rollback(&token).unwrap();
}

#[test]
fn rollback_refuses_a_replaced_group_and_a_foreign_token() {
    let root = mount("replaced");
    let action = action(root.clone());
    let token = action
        .apply_with_policy(&ResctrlActionPolicy { allow: true })
        .unwrap();
    fs::rename(root.join("stutter-cat-t1"), root.join("moved")).unwrap();
    fs::create_dir(root.join("stutter-cat-t1")).unwrap();
    assert!(action.rollback(&token).is_err());
    assert!(
        root.join("stutter-cat-t1").exists(),
        "the other group survives"
    );
    assert!(
        action
            .rollback(&RollbackToken::NiceRestore { records: vec![] })
            .is_err()
    );
}

#[test]
fn recovery_through_the_registry_removes_the_recorded_group() {
    let root = mount("registry");
    let action = action(root.clone());
    let token = action
        .apply_with_policy(&ResctrlActionPolicy { allow: true })
        .unwrap();
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(descriptor.token_kind, "resctrl-group-remove");
    let summary = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap();
    assert_eq!(summary.restored_items, 1);
    assert!(!root.join("stutter-cat-t1").exists());
}

#[test]
fn the_builders_carry_bandwidth_and_tasks_into_the_request() {
    use crate::actions::resctrl_alloc::CatTask;
    let action = ResctrlCatAction::new("b1", 0xf0)
        .with_bandwidth(40)
        .with_tasks(vec![CatTask {
            task: 7,
            start_ticks: 9,
        }]);
    assert_eq!(action.request.mba_percent, Some(40));
    assert_eq!(action.request.tasks.len(), 1);
}
