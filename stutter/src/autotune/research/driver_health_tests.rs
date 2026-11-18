use super::*;

#[test]
fn a_gpu_reset_stops_the_whole_campaign_and_nothing_launches_again() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.polls.push_back(Progress::Failed {
        class: crate::autotune::research::driver::FailureClass::Health,
        code: "GpuReset".into(),
    });
    let Step::Sealed(record) = driver.tick(&mut host, now + 1).unwrap() else {
        panic!("the reset attempt is sealed with its code")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::ClassifiedFailure {
            class: FailureClass::Health,
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

mod production_like {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::autotune::research::gpu_health::{GpuGuard, GpuResetProbe, ResetScan};

    struct SharedProbe(Arc<Mutex<anyhow::Result<ResetScan>>>);
    impl GpuResetProbe for SharedProbe {
        fn resets_since(&mut self, _: u64) -> anyhow::Result<ResetScan> {
            match &*self.0.lock().unwrap() {
                Ok(scan) => Ok(scan.clone()),
                Err(error) => Err(anyhow::anyhow!("{error}")),
            }
        }
    }

    /// A host whose health decisions come from the real `GpuGuard` and its durable files.
    struct GuardHost {
        inner: Fake,
        guard: GpuGuard,
    }

    impl AttemptHost for GuardHost {
        fn health_blocked(&mut self) -> bool {
            self.guard.blocked()
        }
        fn safe_window(&mut self) -> bool {
            self.guard.quiet_for_admission() && self.inner.safe_window()
        }
        fn arm_spec(&mut self, slot: &AttemptSlot) -> anyhow::Result<ArmSpec> {
            self.inner.arm_spec(slot)
        }
        fn launch(
            &mut self,
            slot: &AttemptSlot,
            context: &DispatchContextV1,
        ) -> anyhow::Result<()> {
            anyhow::ensure!(self.guard.admit_now(), "BlockedGpuStability");
            self.guard.begin_attempt();
            self.inner.launch(slot, context)
        }
        fn poll(&mut self) -> Progress {
            self.guard.last_poll = None;
            if self.guard.reset_during_attempt() {
                return Progress::Failed {
                    class: crate::autotune::research::driver::FailureClass::Health,
                    code: "GpuReset".into(),
                };
            }
            self.inner.poll()
        }
        fn abort(&mut self) -> anyhow::Result<()> {
            self.inner.abort()
        }
    }

    fn host(state: &std::path::Path, probe: &Arc<Mutex<anyhow::Result<ResetScan>>>) -> GuardHost {
        GuardHost {
            inner: fake(),
            guard: GpuGuard::new(Box::new(SharedProbe(probe.clone())), state.to_path_buf()),
        }
    }

    #[test]
    fn a_real_guard_stops_the_campaign_after_a_reset_and_after_a_restart_and_for_any_uncertainty() {
        let state = tempfile::tempdir().unwrap();
        let probe = Arc::new(Mutex::new(Ok(ResetScan::quiet())));
        let (_dir, mut driver, intent) = setup(4);
        let mut live = host(state.path(), &probe);
        let now = intent.issued_unix_seconds + 1;
        assert_eq!(driver.tick(&mut live, now).unwrap(), Step::Running(0));
        // The kernel logs a reset while the first arm runs.
        *probe.lock().unwrap() = Ok(ResetScan::events(1, Some(now)));
        let Step::Sealed(record) = driver.tick(&mut live, now + 1).unwrap() else {
            panic!("the reset attempt is sealed")
        };
        assert_eq!(
            record.outcome,
            AttemptOutcome::ClassifiedFailure {
                class: FailureClass::Health,
                code: "GpuReset".into()
            }
        );
        // The journal now reads clean again, the window is open and time passes: still halted.
        *probe.lock().unwrap() = Ok(ResetScan::quiet());
        for step in 2..6 {
            assert_eq!(
                driver.tick(&mut live, now + step).unwrap(),
                Step::Halted("BlockedGpuStability")
            );
        }
        assert_eq!(live.inner.launches, 1);
        // A restarted controller (new guard, new campaign object, same private directory) never
        // launches either.
        let (_dir2, mut again, intent2) = setup(4);
        let mut restarted = host(state.path(), &probe);
        assert_eq!(
            again
                .tick(&mut restarted, intent2.issued_unix_seconds + 1)
                .unwrap(),
            Step::Halted("BlockedGpuStability")
        );
        assert_eq!(restarted.inner.launches, 0);
    }

    #[test]
    fn an_unreadable_journal_never_admits_a_launch() {
        let state = tempfile::tempdir().unwrap();
        let probe: Arc<Mutex<anyhow::Result<ResetScan>>> =
            Arc::new(Mutex::new(Err(anyhow::anyhow!("journal unreadable"))));
        let (_dir, mut driver, intent) = setup(4);
        let mut uncertain = host(state.path(), &probe);
        for step in 1..5 {
            let outcome = driver
                .tick(&mut uncertain, intent.issued_unix_seconds + step)
                .unwrap();
            assert!(
                matches!(outcome, Step::Waiting),
                "unknown health must wait, not launch: {outcome:?}"
            );
        }
        assert_eq!(uncertain.inner.launches, 0);
    }
}
