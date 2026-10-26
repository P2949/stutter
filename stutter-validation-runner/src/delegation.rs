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
}
