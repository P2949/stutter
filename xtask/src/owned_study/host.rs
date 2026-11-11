//! Host-safety inspection for the study: Safe, Busy or Incomplete, and it fails closed.
//!
//! A game or launcher means Busy. Anything this process cannot inspect (an unreadable `/proc`, an
//! entry it may not read, an over-large process table) means Incomplete, never "no game". A process
//! that vanishes while being read, or whose PID is reused between two reads, is skipped.
use std::{fs, io, path::Path};

const MAX_ENTRIES: usize = 200_000;
const MARKERS: [&str; 4] = [
    "SteamLaunch AppId",
    "steamapps/common",
    "gamescope",
    "pressure-vessel",
];
/// Processes whose command line merely MENTIONS a game (a shell, a grep) are not games.
const MENTIONERS: [&str; 6] = ["zsh", "bash", "sh", "grep", "pgrep", "xtask"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostState {
    Safe,
    Busy(String),
    Incomplete(String),
}

fn start_ticks(stat: &str) -> Option<&str> {
    // Field 22; the command name (field 2) may contain spaces and parentheses.
    stat.rsplit_once(") ")?.1.split_whitespace().nth(19)
}

enum Seen {
    Vanished,
    Game(String),
    Plain,
}

fn inspect_process(dir: &Path) -> io::Result<Seen> {
    let read = |name: &str| fs::read(dir.join(name));
    let before = match read("stat") {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Seen::Vanished),
        other => other?,
    };
    let (comm, cmdline) = match (read("comm"), read("cmdline")) {
        (Err(a), _) | (_, Err(a)) if a.kind() == io::ErrorKind::NotFound => {
            return Ok(Seen::Vanished);
        }
        (Err(a), _) | (_, Err(a)) => return Err(a),
        (Ok(comm), Ok(cmdline)) => (comm, cmdline),
    };
    match read("stat") {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Seen::Vanished),
        Err(error) => return Err(error),
        Ok(after) => {
            let (a, b) = (
                String::from_utf8_lossy(&before),
                String::from_utf8_lossy(&after),
            );
            if start_ticks(&a).is_none() || start_ticks(&a) != start_ticks(&b) {
                return Ok(Seen::Vanished);
            }
        }
    }
    let comm = String::from_utf8_lossy(&comm).trim().to_owned();
    let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
    let mentions = MARKERS
        .iter()
        .any(|marker| cmdline.contains(marker) || comm.contains(marker));
    Ok(if mentions && !MENTIONERS.contains(&comm.as_str()) {
        Seen::Game(format!(
            "{comm}: {}",
            cmdline.chars().take(80).collect::<String>()
        ))
    } else {
        Seen::Plain
    })
}

pub fn inspect(proc_root: &Path, own_pid: u32) -> HostState {
    let entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(error) => {
            return HostState::Incomplete(format!("cannot list the process table: {error}"));
        }
    };
    let mut incomplete = None;
    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return HostState::Incomplete(
                "the process table is larger than the inspection bound".into(),
            );
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                incomplete
                    .get_or_insert(format!("a process table entry could not be read: {error}"));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        match inspect_process(&entry.path()) {
            Ok(Seen::Game(what)) => return HostState::Busy(format!("pid {pid} {what}")),
            Ok(Seen::Vanished | Seen::Plain) => {}
            Err(error) => {
                incomplete.get_or_insert(format!("pid {pid} could not be inspected: {error}"));
            }
        }
    }
    incomplete.map_or(HostState::Safe, HostState::Incomplete)
}
