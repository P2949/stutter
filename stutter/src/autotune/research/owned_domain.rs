//! The study-prefix ownership domain of one attempt: processes of this user whose own environment
//! names the study compatdata *and* that started after the attempt's launch request. Steam never
//! hands that path to anything but the research dispatcher's Proton, and Wine and the Steam Runtime
//! reaper inherit it, so these processes stay attributable when the dispatcher has exited or the
//! game has not started yet.
//!
//! Observation is strict. An unlistable `/proc` is an error. A study-path process that predates the
//! attempt is *unattributed*: never signalled, and it blocks settlement. A same-user process that
//! started during the attempt but whose environment cannot be read is *unknown*: never signalled,
//! and it blocks settlement. Only positively attributed processes are ever signalled.

use std::{
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant, UNIX_EPOCH},
};

use stutter_core::ids::Pid;

use super::owned_cleanup::{OwnedProcess, terminate_exact_group};

const MAX_ENVIRON: u64 = 1 << 20;
const MAX_PROC_ENTRIES: usize = 65_536;
const CLOCK_TICKS_PER_SECOND: u64 = 100;
/// A process may begin marginally before the request file's timestamp is taken.
const WINDOW_SLACK_SECONDS: u64 = 2;

/// What an enumeration of the domain found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DomainView {
    /// Study-path processes that started during the attempt: the only ones that may be signalled.
    pub owned: Vec<OwnedProcess>,
    /// Study-path processes that predate the attempt, or same-user processes of the attempt window
    /// whose environment could not be read. Ownership is not provable, so they block settlement.
    pub unattributed: usize,
}

impl DomainView {
    pub fn is_clear(&self) -> bool {
        self.owned.is_empty() && self.unattributed == 0
    }
}

fn stat_fields(proc_root: &Path, pid: u32) -> Option<(u64, bool)> {
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    let mut fields = after.split_whitespace();
    let zombie = fields.next()? == "Z";
    Some((fields.nth(18)?.parse().ok()?, zombie))
}

fn boot_unix_seconds(proc_root: &Path) -> Option<u64> {
    std::fs::read_to_string(proc_root.join("stat"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime ")?.trim().parse().ok())
}

enum Environ {
    Carries,
    Absent,
    Unreadable,
}

fn environ_state(proc_root: &Path, pid: u32, study_compat_data: &Path) -> Environ {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(proc_root.join(pid.to_string()).join("environ")) else {
        return Environ::Unreadable;
    };
    let mut bytes = Vec::new();
    // Read one byte past the cap: an environment that fills it may hold the marker beyond what was
    // captured, so its content is unknown rather than absent.
    if file.take(MAX_ENVIRON + 1).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 > MAX_ENVIRON
    {
        return Environ::Unreadable;
    }
    let wanted = study_compat_data.as_os_str().as_encoded_bytes();
    let carries = bytes.split(|b| *b == 0).any(|entry| {
        entry
            .strip_prefix(b"STEAM_COMPAT_DATA_PATH=")
            .is_some_and(|value| value == wanted)
    });
    if carries {
        Environ::Carries
    } else {
        Environ::Absent
    }
}

/// Enumerate the domain. `window_start_unix` is when the attempt's launch request was recorded.
pub fn observe_domain(
    proc_root: &Path,
    uid: u32,
    study_compat_data: &Path,
    except_pid: u32,
    window_start_unix: u64,
) -> anyhow::Result<DomainView> {
    let boot = boot_unix_seconds(proc_root)
        .ok_or_else(|| anyhow::anyhow!("the boot time is unreadable; the domain is unknown"))?;
    let entries = std::fs::read_dir(proc_root)
        .map_err(|error| anyhow::anyhow!("/proc is unlistable; the domain is unknown: {error}"))?;
    let mut view = DomainView::default();
    let mut scanned = 0usize;
    for entry in entries {
        let name = entry?.file_name();
        let Some(pid) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        scanned += 1;
        anyhow::ensure!(scanned <= MAX_PROC_ENTRIES, "too many processes to scan");
        if pid == except_pid || pid <= 1 {
            continue;
        }
        let dir = proc_root.join(pid.to_string());
        let Ok(meta) = std::fs::metadata(&dir) else {
            continue; // vanished mid-scan
        };
        if meta.uid() != uid {
            continue;
        }
        let Some((ticks, zombie)) = stat_fields(proc_root, pid) else {
            // A process that cannot be fully inspected is not silently dropped: if it carries the
            // study path it is unattributed and blocks settlement. One that vanished does not.
            if dir.exists()
                && matches!(
                    environ_state(proc_root, pid, study_compat_data),
                    Environ::Carries
                )
            {
                view.unattributed += 1;
            }
            continue;
        };
        if zombie {
            continue;
        }
        let started = boot.saturating_add(ticks / CLOCK_TICKS_PER_SECOND);
        let in_window = started.saturating_add(WINDOW_SLACK_SECONDS) >= window_start_unix;
        match environ_state(proc_root, pid, study_compat_data) {
            Environ::Carries if in_window => view.owned.push(OwnedProcess {
                pid: Pid::new(pid),
                start_ticks: ticks,
            }),
            Environ::Carries => view.unattributed += 1,
            Environ::Unreadable if in_window => view.unattributed += 1,
            Environ::Absent | Environ::Unreadable => {}
        }
    }
    Ok(view)
}

/// Terminate every attributed member under ONE monotonic deadline: SIGTERM to all at once, a shared
/// grace period, then SIGKILL to what remains. Unattributed or unknown processes are never signalled
/// and make the result an error, so settlement stays unverified.
pub fn terminate_domain(
    proc_root: &Path,
    uid: u32,
    study_compat_data: &Path,
    except_pid: u32,
    window_start_unix: u64,
    grace: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + grace + Duration::from_secs(3);
    let mut attempts = 0;
    loop {
        let view = observe_domain(
            proc_root,
            uid,
            study_compat_data,
            except_pid,
            window_start_unix,
        )?;
        if view.owned.is_empty() {
            anyhow::ensure!(
                view.unattributed == 0,
                "{} study-prefix processes cannot be attributed to this attempt",
                view.unattributed
            );
            return Ok(());
        }
        attempts += 1;
        anyhow::ensure!(
            attempts <= 3 && Instant::now() < deadline,
            "{} attributed study-prefix processes remain at the cleanup deadline",
            view.owned.len()
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        let term_grace = remaining.min(grace);
        let kill_wait = Duration::from_secs(2).min(remaining.saturating_sub(term_grace));
        terminate_exact_group(proc_root, &view.owned, term_grace, kill_wait)?;
    }
}

/// The attempt window: the launch request's recorded time, from the authenticated dispatch dir.
pub fn window_start_unix(dispatch: &std::fs::File) -> anyhow::Result<u64> {
    use std::os::fd::AsRawFd;
    let path = format!("/proc/self/fd/{}/launch-requested", dispatch.as_raw_fd());
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(modified.duration_since(UNIX_EPOCH)?.as_secs())
}

#[cfg(test)]
mod attribution_tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn an_unlistable_proc_is_unknown_not_empty() {
        assert!(
            observe_domain(Path::new("/definitely/not/proc"), 0, Path::new("/x"), 0, 0).is_err()
        );
    }

    #[test]
    fn a_study_path_process_older_than_the_attempt_is_unattributed_and_never_signalled() {
        let uid = crate::syscall::geteuid();
        let study = Path::new("/nonexistent/stutter-test-old/compatdata");
        let mut old = Command::new("sleep")
            .arg("60")
            .env("STEAM_COMPAT_DATA_PATH", study)
            .spawn()
            .unwrap();
        // The attempt window opens an hour from now, so the running process predates it.
        let future = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let view = observe_domain(Path::new("/proc"), uid, study, 0, future).unwrap();
        assert!(view.owned.is_empty());
        assert_eq!(view.unattributed, 1);
        assert!(
            terminate_domain(
                Path::new("/proc"),
                uid,
                study,
                0,
                future,
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            old.try_wait().unwrap().is_none(),
            "an unattributed process must not be signalled"
        );
        let _ = old.kill();
        let _ = old.wait();
    }

    #[test]
    fn attributed_members_are_terminated_together_under_one_deadline() {
        let uid = crate::syscall::geteuid();
        let study = Path::new("/nonexistent/stutter-test-group/compatdata");
        let spawn = || {
            Command::new("sleep")
                .arg("60")
                .env("STEAM_COMPAT_DATA_PATH", study)
                .spawn()
                .unwrap()
        };
        let mut members: Vec<_> = (0..6).map(|_| spawn()).collect();
        let window = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(60);
        let begun = Instant::now();
        terminate_domain(
            Path::new("/proc"),
            uid,
            study,
            0,
            window,
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(begun.elapsed() < Duration::from_secs(5));
        for member in &mut members {
            assert!(member.wait().is_ok());
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn an_environment_that_fills_the_read_cap_is_unknown_not_absent() {
        let uid = crate::syscall::geteuid();
        let study = Path::new("/nonexistent/stutter-test-cap/compatdata");
        // The marker sits after more than the capped amount of other environment.
        let filler = "x".repeat(60_000);
        let mut command = Command::new("sleep");
        command.arg("60");
        for index in 0..24 {
            command.env(format!("FILL_{index}"), &filler);
        }
        command.env("STEAM_COMPAT_DATA_PATH", study);
        let mut child = command.spawn().unwrap();
        let window = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(60);
        let view = observe_domain(Path::new("/proc"), uid, study, 0, window).unwrap();
        assert!(
            view.owned.is_empty(),
            "a truncated read must not be claimed as owned"
        );
        assert!(
            view.unattributed >= 1,
            "a truncated environment is unknown and blocks settlement"
        );
        let _ = child.kill();
        let _ = child.wait();
    }
}
