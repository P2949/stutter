use super::*;
use crate::autotune::research::{
    arm_selection::screen_fps,
    child_study::{Stage, plan},
    funnel_driver::{FunnelDriver, production_inputs},
};

#[test]
fn the_production_selector_reads_a_screens_sealed_evidence_and_refuses_a_missing_one() {
    let (root, mut runner, study, parent) = attempts(Host::finishing());
    run_to_end(&mut runner, &study);
    let campaign = File::open(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    // The fake host sealed no native results: the screen's evidence directory reads fine and
    // measures nothing, which is not a pair.
    assert!(screen_fps(&campaign, uid, &study).unwrap().is_empty());
    let mut select = production_inputs(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Default::default(),
    );
    let plan = plan(&parent).unwrap();
    let screen = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Screen))
        .unwrap();
    assert_eq!(select(screen).unwrap().arms.len(), parent.tools.len());
    let pilot = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Pilot))
        .unwrap();
    assert!(select(pilot).is_err(), "no measured control and candidate");
    // A screen that never ran has no evidence directory at all.
    let empty = tempfile::tempdir().unwrap();
    std::fs::set_permissions(empty.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(screen_fps(&File::open(empty.path()).unwrap(), uid, screen).is_err());
}

#[test]
fn the_funnels_host_is_reachable_for_the_controllers_observations() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let parent = parent();
    let mut host = Host::finishing();
    host.health = true;
    let attempts = DriverStudyAttempts::new(
        campaign.try_clone().unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(|| 1),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    struct Nothing;
    impl crate::autotune::research::study_executor::ProfileHook for Nothing {
        fn apply(
            &mut self,
            _: crate::autotune::research::child_study::Profile,
        ) -> Result<bool, crate::autotune::research::study_executor::ProfileFailure> {
            Ok(false)
        }
        fn recover(&mut self) -> Result<(), &'static str> {
            Ok(())
        }
        fn restore(&mut self) -> Result<(), &'static str> {
            Ok(())
        }
    }
    let mut driver = FunnelDriver::new(
        &campaign,
        &parent,
        Box::new(Nothing),
        attempts,
        |_| None,
        Default::default(),
    )
    .unwrap();
    assert!(driver.host_mut().health_blocked());
    driver.host_mut().observe_user_input_idle(None);
}
