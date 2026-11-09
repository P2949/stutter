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
}
