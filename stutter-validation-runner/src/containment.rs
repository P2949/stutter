//! Kernel containment for validation children, independent of process groups and environment.
//! Only a fresh cgroup under the caller's kernel-delegated subtree is ever killed. An independent
//! minimal reaper holds its kill fd and observes a private CLOEXEC pipe: owner death closes that
//! pipe even when descendants cleared their environment or detached with setsid.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            net::UnixStream,
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};

#[path = "containment_cleanup.rs"]
mod cleanup;
#[path = "containment_journal.rs"]
mod journal;
#[path = "containment_signal.rs"]
mod signals;

const PREFIX: &str = "stutter-owned-validation-";
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct OwnedCgroup {
    path: PathBuf,
    directory: File,
    heartbeat: Option<UnixStream>,
    reaper: Child,
    finished: bool,
    removed: bool,
    birth: journal::Birth,
}

fn cgroup_file(path: &Path, write: bool, directory: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(!write)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | if directory { libc::O_DIRECTORY } else { 0 })
        .open(path)?;
    validate_cgroup_file(file)
}

fn cgroup_file_at(directory: &File, name: &std::ffi::CStr, write: bool) -> io::Result<File> {
    // SAFETY: directory is an owned fd; name is NUL terminated and openat returns a fresh fd.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if write {
                    libc::O_WRONLY
                } else {
                    libc::O_RDONLY
                },
        )
    };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat returned a fresh descriptor owned by this scope.
    validate_cgroup_file(unsafe { File::from_raw_fd(fd) })
}

fn validate_cgroup_file(file: File) -> io::Result<File> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: file owns a valid fd and stat points to enough writable storage for fstatfs.
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful fstatfs initialized stat above.
    if unsafe { stat.assume_init() }.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a cgroup v2 object",
        ));
    }
    Ok(file)
}

fn start_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// Uses the caller's owned current cgroup without changing ancestor limits or controllers.
pub(crate) fn delegated_root() -> anyhow::Result<PathBuf> {
    let text = fs::read_to_string("/proc/self/cgroup")?;
    let relative = text
        .lines()
        .find_map(|line| line.strip_prefix("0::/"))
        .context("unified cgroup membership unavailable")?;
    ensure!(
        !relative.is_empty()
            && Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "invalid cgroup membership"
    );
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let mount = Path::new("/sys/fs/cgroup");
    let root = mount.join(relative);
    let directory = cgroup_file(&root, false, true)?;
    ensure!(
        directory.metadata()?.uid() == uid,
        "no caller-owned cgroup delegation; refusing uncontainable command"
    );
    // Preserve every existing ancestor resource limit; never move outside the caller's unit.
    Ok(root)
}

pub(crate) fn trusted_shell() -> anyhow::Result<PathBuf> {
    let shell = Path::new("/bin/sh")
        .canonicalize()
        .context("immutable /bin/sh required for validation reaper")?;
    for path in shell.ancestors() {
        let metadata = fs::metadata(path)?;
        ensure!(
            protected_component(metadata.uid(), metadata.mode(), metadata.is_dir()),
            "validation reaper shell must be protected by root-owned ancestors"
        );
    }
    Ok(shell)
}

fn protected_component(uid: u32, mode: u32, directory: bool) -> bool {
    // All entries along the canonical path must be root-owned. A root-owned sticky directory
    // (notably /nix/store) also prevents other owners replacing its existing root-owned child.
    // https://man7.org/linux/man-pages/man2/rename.2.html (EPERM/EACCES, S_ISVTX)
    uid == 0 && (mode & 0o022 == 0 || directory && mode & 0o1000 != 0)
}

impl OwnedCgroup {
    /// Must succeed before a potentially detaching workload is spawned; there is no group-only
    /// fallback. Open descriptors pin this exact cgroup even if a pathname later changes.
    pub fn prepare() -> anyhow::Result<Self> {
        for fd in 0..3 {
            // SAFETY: F_GETFD only checks the supplied standard descriptor.
            ensure!(
                unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1,
                "validation containment requires open standard descriptors"
            );
        }
        let root = delegated_root()?;
        let registry = journal::Registry::open()?;
        cleanup::reconcile_owned(&registry)?;
        let pid = std::process::id();
        let ticks = start_ticks(pid).context("owner starttime unavailable")?;
        let name = format!(
            "{PREFIX}{pid}-{ticks}-{}",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(&name);
        let shell = trusted_shell()?;
        let mut birth = Some(registry.intent(&root, name, pid, ticks)?);
        let mut created = false;
        let prepared = (|| -> anyhow::Result<_> {
            fs::create_dir(&path)
                .context("cannot create owned validation cgroup; refusing command")?;
            created = true;
            let directory = cgroup_file(&path, false, true)?;
            birth
                .as_mut()
                .context("missing validation birth intent")?
                .complete(&directory)?;
            let _procs = cgroup_file_at(&directory, c"cgroup.procs", true)?;
            // Verify the kill interface on this empty group before any workload can start.
            cgroup_file_at(&directory, c"cgroup.kill", true)?.write_all(b"1")?;
            let kill = cgroup_file_at(&directory, c"cgroup.kill", true)?;
            let (reader, writer) = UnixStream::pair()?;
            // The workload must never keep the owner's write end alive after exec.
            // SAFETY: writer owns this valid socket fd; F_GETFD is a read-only query.
            let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) };
            ensure!(
                flags >= 0 && flags & libc::FD_CLOEXEC != 0,
                "validation reaper control pipe must close on exec"
            );
            let reader: OwnedFd = reader.into();
            let kill_fd = kill.as_raw_fd();
            let mut reaper = Command::new(shell);
            reaper
                .args(["-c", "while IFS= read -r line; do :; done; printf 1 >&3"])
                .env_clear()
                .stdin(Stdio::from(reader))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            // SAFETY: only raw async-signal-safe descriptor operations occur after fork. Fd 3
            // was already open in the parent, so it cannot be Rust's exec-error pipe.
            unsafe {
                reaper.pre_exec(move || {
                    if libc::dup2(kill_fd, 3) == -1 || libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let reaper = reaper.spawn().context("start validation cgroup reaper")?;
            Ok(Self {
                path: path.clone(),
                directory,
                heartbeat: Some(writer),
                reaper,
                finished: false,
                removed: false,
                birth: birth.take().context("missing validation birth record")?,
            })
        })();
        if prepared.is_err()
            && (!created || fs::remove_dir(&path).is_ok())
            && let Some(birth) = &birth
        {
            let _ = birth.remove();
        }
        prepared
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Attaches only the new child, before it can execute any workload code. The opened fd
    /// identifies our exact kernel cgroup, and writing 0 names the calling child itself.
    pub fn attach(&self, command: &mut Command) -> io::Result<()> {
        let procs = cgroup_file_at(&self.directory, c"cgroup.procs", true)?;
        // Keep an independent owned fd in Command, even if the guard is dropped before spawn.
        // SAFETY: F_DUPFD_CLOEXEC duplicates an owned descriptor without touching other fds.
        let fd = unsafe { libc::fcntl(procs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fcntl succeeded and returned a fresh owned descriptor above standard fds.
        let procs = unsafe { File::from_raw_fd(fd) };
        // SAFETY: Command owns procs through spawn; only write occurs in the hook.
        unsafe {
            command.pre_exec(move || {
                if libc::write(procs.as_raw_fd(), b"0".as_ptr().cast(), 1) != 1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }

    /// Atomic kernel kill of every descendant, including new sessions and environment-cleared
    /// children. This does not signal any cgroup other than the fresh one this guard created.
    pub fn kill_all(&self) -> io::Result<()> {
        // A fresh opened file has no shared write offset with the independent reaper.
        cgroup_file_at(&self.directory, c"cgroup.kill", true)?.write_all(b"1")
    }

    /// Graceful termination covers detached sessions too; the final kernel kill covers forks
    /// occurring during the grace period. Numeric PID reuse is handled with kernel pidfds.
    pub fn terminate(&self, grace: Duration) -> anyhow::Result<()> {
        signals::terminate(self, grace)
    }

    pub fn populated(&self) -> io::Result<bool> {
        let mut text = String::new();
        let mut events = cgroup_file_at(&self.directory, c"cgroup.events", false)?;
        // Reject a renamed/replaced pathname rather than report a different group's state.
        let current = cgroup_file(&self.path, false, true)?;
        if (current.metadata()?.dev(), current.metadata()?.ino())
            != (
                self.directory.metadata()?.dev(),
                self.directory.metadata()?.ino(),
            )
        {
            return Err(io::Error::other("owned cgroup pathname changed"));
        }
        events.read_to_string(&mut text)?;
        match text
            .lines()
            .find_map(|line| line.strip_prefix("populated "))
        {
            Some("0") => Ok(false),
            Some("1") => Ok(true),
            _ => Err(io::Error::other("invalid cgroup population readback")),
        }
    }

    /// Kill/readback before removing the group. A failed cleanup is an error, never a zero-token
    /// or no-orphan claim. Drop still repeats the kill on every early-return path.
    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }
        if !self.removed {
            self.kill_all()?;
            self.heartbeat.take();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let status = self.reaper.try_wait()?;
                if let Some(status) = status {
                    ensure!(
                        status.success(),
                        "validation cgroup reaper failed: {status}"
                    );
                }
                let exited = status.is_some();
                if exited && !self.populated()? {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "owned validation cgroup cleanup deadline exceeded"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            cleanup::remove_empty(&self.path).context("remove empty owned validation cgroup")?;
            self.removed = true;
        }
        self.birth.remove()?;
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
#[path = "containment_tests.rs"]
mod tests;

impl Drop for OwnedCgroup {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.removed {
            let _ = self.birth.remove();
            return;
        }
        let _ = self.kill_all();
        self.heartbeat.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.reaper.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // If cleanup itself is stalled, preserve the independent reaper instead of killing the
        // only process still able to clean up after a parent error. finish() reports failure.
        if matches!(self.populated(), Ok(false)) && cleanup::remove_empty(&self.path).is_ok() {
            let _ = self.birth.remove();
        }
    }
}
