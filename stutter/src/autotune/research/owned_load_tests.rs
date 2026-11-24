use super::*;

#[test]
fn a_load_is_bounded_in_size_and_time_owned_by_this_process_and_joined_when_stopped() {
    // Refused: no workers, too many, as many as the machine has CPUs, no time, too much time.
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    assert_eq!(
        OwnedLoad::start(0, 5, 100).err(),
        Some("OwnedLoadSizeRefused")
    );
    assert_eq!(
        OwnedLoad::start(MAX_WORKERS + 1, 5, 100).err(),
        Some("OwnedLoadSizeRefused")
    );
    assert_eq!(
        OwnedLoad::start(cpus, 5, 100).err(),
        Some("OwnedLoadSizeRefused")
    );
    assert_eq!(
        OwnedLoad::start(1, 0, 100).err(),
        Some("OwnedLoadDurationRefused")
    );
    assert_eq!(
        OwnedLoad::start(1, MAX_SECONDS + 1, 100).err(),
        Some("OwnedLoadDurationRefused")
    );
    if cpus < 2 {
        return;
    }
    let load = OwnedLoad::start(1, 5, 100).unwrap();
    // The load is a worker of this very process, running until it is stopped.
    assert_eq!(load.alive(), 1);
    let record = load.stop(130);
    assert_eq!(record.pid.as_u32(), std::process::id());
    assert_eq!(record.uid, crate::syscall::geteuid());
    assert_eq!(
        (record.workers, record.started_unix, record.stopped_unix),
        (1, 100, 130)
    );
    assert!(record.all_joined, "every worker was joined");
    assert_ne!(record.start_ticks, 0);
}

#[test]
fn a_load_the_campaign_never_stops_ends_at_its_own_deadline_and_a_dropped_one_is_joined() {
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    if cpus < 2 {
        return;
    }
    // Never stopped: the worker ends by itself at its one-second deadline, and the load did work.
    let load = OwnedLoad::start(1, 1, 100).unwrap();
    let waited = Instant::now();
    while load.alive() > 0 && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(load.alive(), 0, "the worker ended at its own deadline");
    assert!(load.spins() > 0, "the load did real work");
    assert!(load.stop(101).all_joined);
    // Dropped without `stop`: joined by the drop, so no worker outlives its load.
    let load = OwnedLoad::start(1, 5, 100).unwrap();
    let alive = load.alive_handle();
    assert_eq!(alive.load(Ordering::SeqCst), 1);
    drop(load);
    assert_eq!(
        alive.load(Ordering::SeqCst),
        0,
        "the drop joined the worker"
    );
}
