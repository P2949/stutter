use super::*;

struct FixtureSettlementVerifier {
    process_settled: bool,
    worker_settled: bool,
    seal_path: std::path::PathBuf,
    dropped_after_seal: std::sync::Arc<std::sync::atomic::AtomicBool>,
    calls: Option<std::sync::Arc<std::sync::Mutex<(u32, u32)>>>,
    proc_root: Option<std::path::PathBuf>,
}

struct FixtureSettlementGuard {
    seal_path: std::path::PathBuf,
    dropped_after_seal: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ClaimSettlementGuard for FixtureSettlementGuard {}

impl Drop for FixtureSettlementGuard {
    fn drop(&mut self) {
        self.dropped_after_seal.store(
            self.seal_path.is_file(),
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}

impl ClaimSettlementVerifier for FixtureSettlementVerifier {
    fn verify_process(
        &mut self,
        dir: &File,
        context: &DispatchContextV1,
        ticket: &crate::autotune::research::launch_ticket::ResearchLaunchTicketV1,
    ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
        if let Some(calls) = &self.calls {
            calls.lock().unwrap().0 += 1;
        }
        anyhow::ensure!(self.process_settled, "fixture game tree is not settled");
        let proc = self
            .proc_root
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("fixture process source missing"))?;
        anyhow::ensure!(
            crate::autotune::research::dispatch_context::inspect_dispatcher_start(
                dir,
                context,
                ticket.verb,
                proc
            )?
            .is_none(),
            "fixture dispatcher still exists"
        );
        Ok(Box::new(FixtureSettlementGuard {
            seal_path: self.seal_path.clone(),
            dropped_after_seal: self.dropped_after_seal.clone(),
        }))
    }

    fn verify_worker(
        &mut self,
        _context: &DispatchContextV1,
    ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
        if let Some(calls) = &self.calls {
            calls.lock().unwrap().1 += 1;
        }
        anyhow::ensure!(self.worker_settled, "fixture worker token is not settled");
        Ok(Box::new(FixtureSettlementGuard {
            seal_path: self.seal_path.clone(),
            dropped_after_seal: self.dropped_after_seal.clone(),
        }))
    }
}

fn fixture_exited_dispatcher(
    attempt: &std::path::Path,
    proc: &std::path::Path,
    ticket: &crate::autotune::research::launch_ticket::ResearchLaunchTicketV1,
) {
    use crate::autotune::research::dispatch_context::DispatcherStartV1;
    std::fs::create_dir_all(proc.join("sys/kernel/random")).unwrap();
    std::fs::write(proc.join("sys/kernel/random/boot_id"), &ticket.boot_id).unwrap();
    let record = DispatcherStartV1 {
        schema_version: 1,
        ticket_id: ticket.ticket_id.clone(),
        intent_sha256: ticket.intent_sha256.clone(),
        attempt_sha256: ticket.attempt_sha256.clone(),
        verb: ticket.verb,
        uid: ticket.uid,
        boot_id: ticket.boot_id.clone(),
        observed_unix_seconds: ticket.issued_unix_seconds,
        pid: stutter_core::ids::Pid::new(123),
        start_ticks: 42,
        pid_namespace_device: 1,
        pid_namespace_inode: 1,
    };
    let dir = File::from(crate::syscall::open_dir_no_symlinks(attempt).unwrap());
    crate::autotune::research::report_store::write_new(
        &dir,
        &format!("dispatcher-{}.json", ticket.verb.as_steam_argument()),
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn consumed_ticket_seals_only_with_independent_settlement_guards() {
    use crate::autotune::research::launch_ticket::{SteamVerb, binding, consume_launch_ticket};
    for (process_settled, worker_settled) in [(false, true), (true, false), (true, true)] {
        let (dir, mut driver, intent) = setup(4);
        let mut host = fake();
        host.auto_finish = true;
        let now = intent.issued_unix_seconds + 1;
        assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
        let context = driver.armed_context.clone().unwrap();
        let ticket_id = context.tickets.get("run").unwrap();
        let attempt = dir.path().join("dispatch/0000");
        let binding = binding(&context.intent, &context.slot, SteamVerb::Run).unwrap();
        let claimed = consume_launch_ticket(
            crate::syscall::open_dir_no_symlinks(&attempt).unwrap(),
            ticket_id,
            &binding,
            now,
        )
        .unwrap();
        let proc = tempfile::tempdir().unwrap();
        fixture_exited_dispatcher(&attempt, proc.path(), &claimed);
        let dropped_after_seal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seal_path = dir.path().join("journal/0000.json");
        driver.set_claim_verifier(Box::new(FixtureSettlementVerifier {
            process_settled,
            worker_settled,
            seal_path: seal_path.clone(),
            dropped_after_seal: dropped_after_seal.clone(),
            calls: None,
            proc_root: Some(proc.path().to_path_buf()),
        }));
        let result = driver.tick(&mut host, now + 1);
        assert_eq!(result.is_ok(), process_settled && worker_settled);
        assert_eq!(seal_path.is_file(), process_settled && worker_settled);
        if process_settled && worker_settled {
            assert!(matches!(result.unwrap(), Step::Sealed(_)));
            assert!(dropped_after_seal.load(std::sync::atomic::Ordering::SeqCst));
            let other = context.tickets.get("waitforexitandrun").unwrap();
            assert_eq!(
                std::fs::read(attempt.join(format!("used-{other}"))).unwrap(),
                b"revoked-v1"
            );
        } else {
            drop(driver);
            let reopened = ResearchJournal::open(
                crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
                intent.uid,
                build_schedule(&intent).unwrap(),
            );
            assert!(format!("{:#}", reopened.err().unwrap()).contains("UnsafeToResume"));
        }
    }
}

#[test]
fn both_consumed_steam_verbs_share_one_worker_settlement_guard() {
    use crate::autotune::research::launch_ticket::{SteamVerb, binding, consume_launch_ticket};
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let context = driver.armed_context.clone().unwrap();
    let attempt = dir.path().join("dispatch/0000");
    let proc = tempfile::tempdir().unwrap();
    for verb in [SteamVerb::Run, SteamVerb::WaitForExitAndRun] {
        let ticket_id = context.tickets.get(verb.as_steam_argument()).unwrap();
        let expected = binding(&context.intent, &context.slot, verb).unwrap();
        let claimed = consume_launch_ticket(
            crate::syscall::open_dir_no_symlinks(&attempt).unwrap(),
            ticket_id,
            &expected,
            now,
        )
        .unwrap();
        fixture_exited_dispatcher(&attempt, proc.path(), &claimed);
    }
    let calls = std::sync::Arc::new(std::sync::Mutex::new((0, 0)));
    let dropped_after_seal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    driver.set_claim_verifier(Box::new(FixtureSettlementVerifier {
        process_settled: true,
        worker_settled: true,
        seal_path: dir.path().join("journal/0000.json"),
        dropped_after_seal: dropped_after_seal.clone(),
        calls: Some(calls.clone()),
        proc_root: Some(proc.path().to_path_buf()),
    }));
    assert!(matches!(
        driver.tick(&mut host, now + 1).unwrap(),
        Step::Sealed(_)
    ));
    assert_eq!(*calls.lock().unwrap(), (2, 1));
    assert!(dropped_after_seal.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn forged_consumed_ticket_cannot_use_settlement_verifier() {
    use crate::autotune::research::launch_ticket::{SteamVerb, binding, consume_launch_ticket};
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let context = driver.armed_context.clone().unwrap();
    let ticket_id = context.tickets.get("run").unwrap();
    let attempt = dir.path().join("dispatch/0000");
    let expected = binding(&context.intent, &context.slot, SteamVerb::Run).unwrap();
    let mut ticket = consume_launch_ticket(
        crate::syscall::open_dir_no_symlinks(&attempt).unwrap(),
        ticket_id,
        &expected,
        now,
    )
    .unwrap();
    ticket.game_build_id += 1;
    let bytes = serde_json::to_vec(&ticket).unwrap();
    std::fs::write(attempt.join(format!("ticket-{ticket_id}.json")), &bytes).unwrap();
    std::fs::write(
        attempt.join(format!("used-{ticket_id}")),
        stutter_core::hash::sha256_hex(&bytes),
    )
    .unwrap();
    let seal_path = dir.path().join("journal/0000.json");
    let dropped_after_seal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    driver.set_claim_verifier(Box::new(FixtureSettlementVerifier {
        process_settled: true,
        worker_settled: true,
        seal_path: seal_path.clone(),
        dropped_after_seal,
        calls: None,
        proc_root: None,
    }));
    assert!(driver.tick(&mut host, now + 1).is_err());
    assert!(!seal_path.exists());
}

#[test]
fn genuine_claim_without_settlement_verifier_remains_unsealed() {
    use crate::autotune::research::launch_ticket::{SteamVerb, binding, consume_launch_ticket};
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let context = driver.armed_context.clone().unwrap();
    let attempt = dir.path().join("dispatch/0000");
    let ticket_id = context.tickets.get("run").unwrap();
    let binding = binding(&context.intent, &context.slot, SteamVerb::Run).unwrap();
    consume_launch_ticket(
        crate::syscall::open_dir_no_symlinks(&attempt).unwrap(),
        ticket_id,
        &binding,
        now,
    )
    .unwrap();
    assert!(driver.tick(&mut host, now + 1).is_err());
    assert!(!dir.path().join("journal/0000.json").exists());
    let unused = context.tickets.get("waitforexitandrun").unwrap();
    assert!(!attempt.join(format!("used-{unused}")).exists());
}
