//! A delegated cgroup subtree for the campaign user.
//!
//! Moving tasks between cgroups, creating them and writing `cpuset`/`cpu` files are privileged on a
//! host whose suites live in the runner's own service cgroup. A suite that validates cgroup
//! placement therefore runs inside a subtree the runner creates, owns for the campaign user (the
//! kernel's delegation model: the directory and its `cgroup.*` files are handed over) and removes
//! afterwards. Nothing outside that subtree is ever touched, and no path comes from the caller.
use std::{
    fs, io,
    os::unix::fs::chown,
    path::{Path, PathBuf},
};

/// Controllers the delegated subtree offers its owner.
const CONTROLLERS: &str = "+cpu +cpuset";

fn subtree_name(campaign_id: &str) -> String {
    format!(
        "stutter-validation-{}",
        campaign_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            })
            .collect::<String>()
    )
}

fn give_to_owner(path: &Path, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    if uid.is_some() || gid.is_some() {
        chown(path, uid, gid)?;
    }
    Ok(())
}

/// Creates (idempotently) `<cgroup_mount>/stutter-validation-<campaign>/suite`, delegated to the
/// owner, and returns the leaf a suite process is attached to. `owner` is `None` when the runner
/// already runs as the campaign user (tests).
pub fn prepare(
    cgroup_mount: &Path,
    campaign_id: &str,
    owner: Option<(u32, u32)>,
) -> io::Result<PathBuf> {
    let (uid, gid) = (owner.map(|o| o.0), owner.map(|o| o.1));
    // The parent must offer the controllers before a child may; ignore "already enabled".
    let _ = fs::write(cgroup_mount.join("cgroup.subtree_control"), CONTROLLERS);
    let root = cgroup_mount.join(subtree_name(campaign_id));
    if !root.is_dir() {
        fs::create_dir(&root)?;
    }
    // A cgroup with processes cannot enable controllers for children, so the root stays empty and
    // every process lives in a leaf.
    let _ = fs::write(root.join("cgroup.subtree_control"), CONTROLLERS);
    give_to_owner(&root, uid, gid)?;
    for entry in fs::read_dir(&root)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let delegable = name == "cgroup.procs"
            || name == "cgroup.threads"
            || name == "cgroup.subtree_control"
            || name.starts_with("cpuset.")
            || name.starts_with("cpu.");
        if delegable && entry.path().is_file() {
            give_to_owner(&entry.path(), uid, gid)?;
        }
    }
    let leaf = root.join("suite");
    if !leaf.is_dir() {
        fs::create_dir(&leaf)?;
    }
    give_to_owner(&leaf, uid, gid)?;
    for file in ["cgroup.procs", "cgroup.threads"] {
        let path = leaf.join(file);
        if path.is_file() {
            give_to_owner(&path, uid, gid)?;
        }
    }
    Ok(leaf)
}

/// Moves `pid` into the leaf (the runner is privileged; the suite is not yet running anything).
pub fn attach(leaf: &Path, pid: u32) -> io::Result<()> {
    fs::write(leaf.join("cgroup.procs"), pid.to_string())
}

/// How long a killed subtree may take to empty before the run is failed.
const EMPTY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// True while any cgroup in the subtree still has a task.
fn populated(dir: &Path) -> io::Result<bool> {
    if !fs::read_to_string(dir.join("cgroup.procs"))
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Ok(true);
    }
    for entry in fs::read_dir(dir)?.flatten() {
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir && populated(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Ends everything the suite left in its delegated subtree and proves it is gone.
///
/// The subtree is killed as one unit through the root-owned `cgroup.kill`, so a `setsid`
/// grandchild, a TERM-ignoring process or a cgroup the suite created itself cannot survive; the
/// process group, which the child may have left, plays no part. The subtree is then required to be
/// empty and is removed. A subtree that stays populated, or cannot be removed, is an error the
/// caller must fail the run on: a leaked descendant is never reported as a clean run. Only the
/// subtree named by the campaign id is touched, never a path from the caller.
pub fn contain_end(cgroup_mount: &Path, campaign_id: &str) -> io::Result<()> {
    contain_end_within(cgroup_mount, campaign_id, EMPTY_DEADLINE)
}

fn contain_end_within(
    cgroup_mount: &Path,
    campaign_id: &str,
    allowed: std::time::Duration,
) -> io::Result<()> {
    let root = cgroup_mount.join(subtree_name(campaign_id));
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(meta) if !meta.is_dir() => {
            return Err(io::Error::other(format!(
                "{} is not a directory; refusing to touch it",
                root.display()
            )));
        }
        Ok(_) => {}
    }
    let deadline = std::time::Instant::now() + allowed;
    loop {
        fs::write(root.join("cgroup.kill"), "1")?;
        if !populated(&root)? {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "{} still has tasks after cgroup.kill; leaked descendants not proven gone",
                root.display()
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    remove(cgroup_mount, campaign_id)
}

/// Removes the campaign's subtree, children first. Cgroups with live tasks stay (and are reported
/// by the error), so a leaked task is never silently abandoned.
pub fn remove(cgroup_mount: &Path, campaign_id: &str) -> io::Result<()> {
    let root = cgroup_mount.join(subtree_name(campaign_id));
    if !root.is_dir() {
        return Ok(());
    }
    fn walk(dir: &Path) -> io::Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            if entry.path().is_dir() {
                walk(&entry.path())?;
            }
        }
        fs::remove_dir(dir)
    }
    walk(&root)
}

/// An independent process that kills the whole delegated subtree when its owner dies.
///
/// It holds an open `cgroup.kill` descriptor for the exact subtree root (so a later rename or
/// replacement of the path changes nothing it can reach) and reads a private close-on-exec
/// heartbeat pipe. When the owner exits for any reason, including `SIGKILL`, the pipe closes, the
/// reaper writes `1` to the kill descriptor and the kernel ends every task in the subtree, whatever
/// session it started or environment it cleared. Nothing in the owner has to run for this to
/// happen. It is started before any suite process can exist, and a subtree that cannot be given
/// one is not used.
pub struct DelegatedReaper {
    reaper: std::process::Child,
    heartbeat: Option<std::os::unix::net::UnixStream>,
}

impl DelegatedReaper {
    pub fn start(cgroup_mount: &Path, campaign_id: &str) -> anyhow::Result<Self> {
        use std::os::{
            fd::{AsRawFd, OwnedFd},
            unix::{fs::OpenOptionsExt, net::UnixStream, process::CommandExt},
        };
        let root = cgroup_mount.join(subtree_name(campaign_id));
        anyhow::ensure!(
            fs::symlink_metadata(&root).is_ok_and(|meta| meta.is_dir()),
            "the delegated subtree does not exist; refusing to start a reaper"
        );
        let kill = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("cgroup.kill"))?;
        anyhow::ensure!(
            kill.metadata()?.is_file(),
            "cgroup.kill is not a regular control file"
        );
        let (reader, writer) = UnixStream::pair()?;
        // The suite must never inherit the owner's write end.
        // SAFETY: F_GETFD only queries a descriptor this function owns.
        let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) };
        anyhow::ensure!(
            flags >= 0 && flags & libc::FD_CLOEXEC != 0,
            "the reaper heartbeat must close on exec"
        );
        let reader: OwnedFd = reader.into();
        let kill_fd = kill.as_raw_fd();
        let mut command = std::process::Command::new(crate::containment::trusted_shell()?);
        command
            .args(["-c", "while IFS= read -r line; do :; done; printf 1 >&3"])
            .env_clear()
            .stdin(std::process::Stdio::from(reader))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        // SAFETY: only async-signal-safe descriptor operations run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(kill_fd, 3) == -1 || libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let reaper = command.spawn()?;
        Ok(Self {
            reaper,
            heartbeat: Some(writer),
        })
    }

    /// Hands the heartbeat write end to the process whose death should trigger the kill (the
    /// runner keeps it; tests give it to a separate owner they can `SIGKILL`).
    pub fn take_heartbeat(&mut self) -> Option<std::os::fd::OwnedFd> {
        self.heartbeat.take().map(Into::into)
    }

    /// Ends the reaper once the subtree was contained and removed in the ordinary way.
    pub fn finish(mut self) {
        self.heartbeat = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if matches!(self.reaper.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.reaper.kill();
        let _ = self.reaper.wait();
    }
}

impl Drop for DelegatedReaper {
    fn drop(&mut self) {
        // Dropping without `finish` is an owner exit: the heartbeat closes and the kill fires.
        self.heartbeat = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subtree_name_is_derived_from_the_campaign_id_only() {
        assert_eq!(
            subtree_name("mut-nice-0c1f"),
            "stutter-validation-mut-nice-0c1f"
        );
        assert_eq!(subtree_name("a/../b c"), "stutter-validation-a----b-c");
    }

    #[test]
    fn prepare_creates_a_leaf_inside_the_campaign_subtree_and_remove_deletes_it() {
        let mount = std::env::temp_dir().join(format!("stutter-delegation-{}", std::process::id()));
        fs::create_dir_all(&mount).unwrap();
        let leaf = prepare(&mount, "campaign-1", None).unwrap();
        assert_eq!(leaf, mount.join("stutter-validation-campaign-1/suite"));
        assert!(leaf.is_dir());
        // Idempotent.
        assert_eq!(prepare(&mount, "campaign-1", None).unwrap(), leaf);
        // A plain directory tree stands in for the cgroup filesystem; the control files `prepare`
        // wrote into it are real files here, whereas cgroupfs removes its own with the directory.
        for file in ["cgroup.subtree_control", "suite/cgroup.procs"] {
            let _ = fs::remove_file(mount.join("stutter-validation-campaign-1").join(file));
        }
        let _ = fs::remove_file(mount.join("cgroup.subtree_control"));
        remove(&mount, "campaign-1").unwrap();
        assert!(!mount.join("stutter-validation-campaign-1").exists());
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    fn containment_end_of_an_absent_subtree_is_a_no_op_and_a_non_directory_is_refused() {
        let mount = std::env::temp_dir().join(format!("stutter-deleg-end-{}", std::process::id()));
        fs::create_dir_all(&mount).unwrap();
        contain_end(&mount, "never-created").unwrap();
        std::os::unix::fs::symlink("/", mount.join(subtree_name("linked"))).unwrap();
        let error = contain_end(&mount, "linked").unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
        assert!(Path::new("/").is_dir(), "the link target was not touched");
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    fn a_subtree_that_stays_populated_is_an_error_and_is_not_removed() {
        let mount = std::env::temp_dir().join(format!("stutter-deleg-pop-{}", std::process::id()));
        fs::create_dir_all(&mount).unwrap();
        let leaf = prepare(&mount, "busy", None).unwrap();
        // A task the kill cannot take away, as in a leaked descendant in a nested cgroup.
        let nested = leaf.join("child-made-by-the-suite");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("cgroup.procs"), "424242\n").unwrap();
        assert!(populated(&mount.join(subtree_name("busy"))).unwrap());
        fs::remove_file(nested.join("cgroup.procs")).unwrap();
        assert!(!populated(&mount.join(subtree_name("busy"))).unwrap());
        fs::write(nested.join("cgroup.procs"), "424242\n").unwrap();
        let root = mount.join(subtree_name("busy"));
        let error =
            contain_end_within(&mount, "busy", std::time::Duration::from_millis(120)).unwrap_err();
        assert!(error.to_string().contains("still has tasks"), "{error}");
        assert!(
            nested.is_dir() && root.is_dir(),
            "a populated subtree is left in place, never forcibly deleted"
        );
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    #[ignore = "live disposable kernel cgroup; requires caller-owned delegation"]
    fn live_containment_end_kills_a_setsid_grandchild_and_spares_an_unrelated_process() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::Duration,
        };
        let mount = crate::containment::delegated_root()
            .expect("BlockedResource: caller-owned cgroup delegation required");
        let id = format!("live-deleg-{}", std::process::id());
        let leaf = prepare(&mount, &id, None).unwrap();
        // The grandchild leaves the process group and ignores TERM, so only the subtree reaches it.
        let mut child = Command::new("sh")
            .args([
                "-c",
                "read go; setsid sh -c 'trap \"\" TERM; exec sleep 600' & sleep 600",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        attach(&leaf, child.id()).unwrap();
        child.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
        let mut sentinel = Command::new("sleep").arg("600").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let root = mount.join(subtree_name(&id));
        assert!(populated(&root).unwrap(), "the suite left nothing running");
        let started = std::time::Instant::now();
        contain_end(&mount, &id).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!root.exists(), "the delegated subtree was not removed");
        let _ = child.wait();
        assert!(
            sentinel.try_wait().unwrap().is_none(),
            "an unrelated process was killed"
        );
        sentinel.kill().unwrap();
        sentinel.wait().unwrap();
    }

    fn fake_subtree(name: &str) -> (PathBuf, PathBuf) {
        let mount =
            std::env::temp_dir().join(format!("stutter-reaper-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&mount);
        fs::create_dir_all(&mount).unwrap();
        let root = mount.join(subtree_name("campaign"));
        fs::create_dir_all(root.join("suite")).unwrap();
        fs::write(root.join("cgroup.kill"), "").unwrap();
        (mount, root)
    }

    #[test]
    fn the_reaper_kills_the_exact_subtree_when_its_owner_goes_away_and_not_before() {
        let (mount, root) = fake_subtree("fires");
        let mut reaper = DelegatedReaper::start(&mount, "campaign").unwrap();
        let heartbeat = reaper.take_heartbeat().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            fs::read_to_string(root.join("cgroup.kill")).unwrap(),
            "",
            "the kill fired while the owner was alive"
        );
        drop(heartbeat); // the owner exits
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while fs::read_to_string(root.join("cgroup.kill")).unwrap() != "1" {
            assert!(
                std::time::Instant::now() < deadline,
                "the reaper never fired"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        reaper.finish();
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    fn a_subtree_without_a_kill_file_or_that_is_not_a_directory_gets_no_reaper() {
        let mount =
            std::env::temp_dir().join(format!("stutter-reaper-none-{}", std::process::id()));
        fs::create_dir_all(&mount).unwrap();
        assert!(DelegatedReaper::start(&mount, "absent").is_err());
        fs::create_dir_all(mount.join(subtree_name("nokill"))).unwrap();
        assert!(
            DelegatedReaper::start(&mount, "nokill").is_err(),
            "a subtree whose cgroup.kill cannot be opened must not be used"
        );
        std::os::unix::fs::symlink("/", mount.join(subtree_name("linked"))).unwrap();
        assert!(DelegatedReaper::start(&mount, "linked").is_err());
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    #[ignore = "live disposable kernel cgroup; requires caller-owned delegation"]
    fn live_owner_sigkill_leaves_no_descendant_in_the_delegated_subtree() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let mount = crate::containment::delegated_root()
            .expect("BlockedResource: caller-owned cgroup delegation required");
        let id = format!("live-reaper-{}", std::process::id());
        let leaf = prepare(&mount, &id, None).unwrap();
        let mut reaper = DelegatedReaper::start(&mount, &id).unwrap();
        // The "runner" is a separate process that holds the heartbeat; killing it is the crash.
        let mut owner = Command::new("sleep")
            .arg("600")
            .stdin(Stdio::from(reaper.take_heartbeat().unwrap()))
            .spawn()
            .unwrap();
        let mut suite = Command::new("sh")
            .args([
                "-c",
                "read go; setsid sh -c 'trap \"\" TERM; exec sleep 600' & sleep 600",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        attach(&leaf, suite.id()).unwrap();
        suite.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
        let mut sentinel = Command::new("sleep").arg("600").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let root = mount.join(subtree_name(&id));
        assert!(populated(&root).unwrap(), "the suite left nothing running");
        owner.kill().unwrap(); // SIGKILL: nothing in the owner runs any cleanup
        owner.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while populated(&root).unwrap() {
            assert!(
                Instant::now() < deadline,
                "a descendant survived the owner's death"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = suite.wait();
        assert!(
            sentinel.try_wait().unwrap().is_none(),
            "an unrelated process was killed"
        );
        sentinel.kill().unwrap();
        sentinel.wait().unwrap();
        // A later ordinary cleanup finds it already empty and removes the directory.
        contain_end(&mount, &id).unwrap();
        assert!(!root.exists());
        reaper.finish();
    }
}
