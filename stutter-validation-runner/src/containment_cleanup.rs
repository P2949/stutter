//! Bounded crash-resume cleanup of this caller's explicitly named cgroup namespace.
use super::*;

pub(super) fn parse_owner(name: &str) -> Option<(u32, u64)> {
    let mut fields = name.strip_prefix(PREFIX)?.split('-');
    let pid = fields.next()?.parse().ok()?;
    let ticks = fields.next()?.parse().ok()?;
    let sequence: u64 = fields.next()?.parse().ok()?;
    (fields.next().is_none()
        && pid > 1
        && ticks > 0
        && sequence > 0
        && name == format!("{PREFIX}{pid}-{ticks}-{sequence}"))
    .then_some((pid, ticks))
}

pub(super) fn owner_gone(pid: u32, ticks: u64) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(19))
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|current| current != ticks),
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

pub(super) fn reconcile_owned(registry: &journal::Registry) -> anyhow::Result<()> {
    for record in registry.records()? {
        if record.boot != registry.boot {
            registry.remove(&record)?;
            continue;
        }
        if !owner_gone(record.pid, record.ticks) {
            continue;
        }
        let parent = match cgroup_file(&record.parent, false, true) {
            Ok(parent) => parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                registry.remove(&record)?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            parent.metadata()?.uid() == registry.uid
                && journal::KernelId::of(&parent)? == record.parent_id,
            "validation scope parent identity changed; refusing cleanup"
        );
        let path = record.parent.join(&record.name);
        let directory = match cgroup_file(&path, false, true) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                registry.remove(&record)?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            directory.metadata()?.uid() == registry.uid,
            "stale validation cgroup has another owner"
        );
        if let Some(expected) = record.resource {
            ensure!(
                journal::KernelId::of(&directory)? == expected,
                "stale validation cgroup inode changed; refusing cleanup"
            );
            cgroup_file_at(&directory, c"cgroup.kill", true)?.write_all(b"1")?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut events = String::new();
            cgroup_file_at(&directory, c"cgroup.events", false)?.read_to_string(&mut events)?;
            match events
                .lines()
                .find_map(|line| line.strip_prefix("populated "))
            {
                Some("0") => break,
                Some("1") => {
                    ensure!(
                        record.resource.is_some(),
                        "unproven validation creation intent is populated; refusing cleanup"
                    );
                }
                _ => anyhow::bail!("invalid stale validation cgroup population"),
            }
            ensure!(
                Instant::now() < deadline,
                "stale validation cgroup did not empty"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        match remove_empty(&path) {
            Ok(()) => (),
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
        registry.remove(&record)?;
    }
    Ok(())
}

pub(super) fn remove_empty(path: &Path) -> anyhow::Result<()> {
    fn walk(path: &Path, depth: usize, budget: &mut usize) -> anyhow::Result<()> {
        ensure!(
            depth <= 64 && *budget > 0,
            "validation cgroup cleanup tree bound exceeded"
        );
        *budget -= 1;
        // Reject a regular filesystem or a symlink before walking/removing anything.
        let _directory = cgroup_file(path, false, true)?;
        for (index, entry) in fs::read_dir(path)?.enumerate() {
            ensure!(index < 1024, "validation cgroup file bound exceeded");
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), depth + 1, budget)?;
            }
        }
        // The kernel refuses a populated cgroup, so a concurrent attachment is an error.
        fs::remove_dir(path)?;
        Ok(())
    }
    walk(path, 0, &mut 1024)
}
