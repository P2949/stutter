//! Verify helper descendants terminate when their parent dies, independently
//! of affinity mutation or elevated privileges.
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn identity(pid: u32) -> Option<(u64, String)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some((fields.get(19)?.parse().ok()?, fields.first()?.to_string()))
}
#[test]
fn disposable_descendants_exit_after_parent_is_killed() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("stutter-workload-cleanup-{suffix}"));
    fs::create_dir(&root).unwrap();
    let ready = root.join("ready.json");
    let mut parent = Owned(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args([
                "live-workload",
                "--descendants",
                "--seconds",
                "3",
                "--ready",
            ])
            .arg(&ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            parent.0.try_wait().unwrap().is_none(),
            "helper exited before readiness"
        );
        assert!(Instant::now() < deadline, "helper readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = parent.0.id();
    let children = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap()
        .split_whitespace()
        .map(|pid| pid.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    let identities = children
        .iter()
        .map(|pid| (*pid, identity(*pid).unwrap().0))
        .collect::<Vec<_>>();
    parent.0.kill().unwrap();
    parent.0.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let running = identities.iter().any(|(pid, ticks)| {
            identity(*pid).is_some_and(|(now, state)| now == *ticks && state != "Z")
        });
        if !running {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "descendant survived parent death"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(root).unwrap();
}
