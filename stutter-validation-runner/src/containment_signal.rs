use super::*;

fn collect(
    directory: &File,
    path: &Path,
    prefix: &str,
    depth: usize,
    remaining: &mut usize,
    handles: &mut Vec<OwnedFd>,
) -> anyhow::Result<()> {
    ensure!(
        depth <= 64 && *remaining > 0,
        "validation process tree bound exceeded"
    );
    *remaining -= 1;
    let mut text = String::new();
    cgroup_file_at(directory, c"cgroup.procs", false)?
        .take(131073)
        .read_to_string(&mut text)?;
    ensure!(text.len() <= 131072, "validation process list too large");
    for line in text.lines() {
        ensure!(handles.len() < 32768, "validation pidfd bound exceeded");
        let pid: u32 = line.parse()?;
        ensure!(
            pid > 1 && pid != std::process::id(),
            "invalid validation child identity"
        );
        // SAFETY: pidfd_open takes a numeric pid and zero flags and returns a fresh descriptor.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error.into());
        }
        // SAFETY: successful pidfd_open returned a fresh descriptor owned by this scope.
        let handle = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        // Open identity first, then verify membership. A reused numeric pid outside this scope
        // is never signalled; a later exit cannot redirect a signal to a replacement process.
        let Ok(membership) = fs::read_to_string(format!("/proc/{pid}/cgroup")) else {
            continue;
        };
        let belongs = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .is_some_and(|member| {
                member == prefix
                    || member
                        .strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with('/'))
            });
        if belongs {
            handles.push(handle);
        }
    }
    for (index, entry) in fs::read_dir(path)?.enumerate() {
        ensure!(index < 1024, "validation cgroup entry bound exceeded");
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let child = cgroup_file(&entry.path(), false, true)?;
            collect(&child, &entry.path(), prefix, depth + 1, remaining, handles)?;
        }
    }
    Ok(())
}

pub(super) fn terminate(scope: &OwnedCgroup, grace: Duration) -> anyhow::Result<()> {
    // populated() also refuses a renamed/replaced path before any pidfd signals are sent.
    if !scope.populated()? {
        return Ok(());
    }
    let relative = scope.path.strip_prefix("/sys/fs/cgroup")?;
    let prefix = format!("/{}", relative.display());
    let mut handles = Vec::new();
    collect(
        &scope.directory,
        &scope.path,
        &prefix,
        0,
        &mut 1024,
        &mut handles,
    )?;
    for handle in handles {
        // SAFETY: handle is a live owned pidfd; there is no siginfo or flag payload.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                handle.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
    }
    let deadline = Instant::now() + grace;
    while scope.populated()? && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    scope.kill_all()?;
    Ok(())
}
