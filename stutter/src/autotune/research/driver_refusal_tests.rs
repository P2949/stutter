use super::*;
use crate::autotune::research::steam_host::StopStep;

/// A refused launch is stopped one bounded step per tick: while the launched tree is not yet proven gone the attempt
/// is reported running and nothing is sealed, and the refusal is sealed on the tick that proves it settled.
#[test]
fn a_refused_launch_is_sealed_only_once_its_stop_settles_across_ticks() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.launch_fails = true;
    host.abort_steps = VecDeque::from([StopStep::Pending, StopStep::Settled]);
    let now = intent.issued_unix_seconds + 1;
    assert!(matches!(
        driver.tick(&mut host, now).unwrap(),
        Step::Running(0)
    ));
    assert!(driver.abort_pending());
    assert!(matches!(
        driver.tick(&mut host, now + 1).unwrap(),
        Step::Sealed(_)
    ));
    assert!(!driver.abort_pending());
    assert_eq!(host.aborts, 0, "the blocking stop is never used");
}
