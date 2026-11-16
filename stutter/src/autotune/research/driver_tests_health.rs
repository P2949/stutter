use super::*;

#[test]
fn a_gpu_reset_stops_the_whole_campaign_and_nothing_launches_again() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.polls.push_back(Progress::Failed {
        code: "GpuReset".into(),
    });
    let Step::Sealed(record) = driver.tick(&mut host, now + 1).unwrap() else {
        panic!("the reset attempt is sealed with its code")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "GpuReset".into()
        }
    );
    // The host's durable marker now exists. Schedule, budget and deadline all remain, and the
    // safe window is open: the campaign still never launches another arm.
    host.health_stop = true;
    for step in 2..8 {
        assert_eq!(
            driver.tick(&mut host, now + step).unwrap(),
            Step::Halted("BlockedGpuStability")
        );
    }
    assert_eq!(host.launches, 1);
}

#[test]
fn a_durable_health_stop_blocks_the_first_launch_after_a_controller_restart() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.health_stop = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(
        driver.tick(&mut host, now).unwrap(),
        Step::Halted("BlockedGpuStability")
    );
    assert_eq!(host.launches, 0);
}
