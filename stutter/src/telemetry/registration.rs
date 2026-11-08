//! Local-only registration of a launch with the telemetry service.
//!
//! The socket accepts three verbs only: register, cancel and inspect. It carries no mutation
//! authority, and a registration is correlation data: it can raise a source to `LaunchBound` at
//! most, never to `ValidatedDirect`. The peer's UID comes from the kernel (`SO_PEERCRED`), not
//! from anything the peer says.
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::FileTypeExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::session::{ConfinedDir, ConfinementError};

/// Most simultaneous registrations; more are refused rather than growing without bound.
pub const MAX_REGISTRATIONS: usize = 64;
/// Longest accepted request line, in bytes.
pub const MAX_LINE_BYTES: u64 = 256;
/// Longest accepted launch identifier.
pub const MAX_LAUNCH_ID_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
}

pub fn kernel_peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let (pid, uid, _) = crate::syscall::unix_peer_credentials(stream)?;
    Ok(PeerCredentials { pid, uid })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub launch_id: String,
    pub pid: u32,
    pub uid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Registered,
    Cancelled,
    Inspected(Vec<Registration>),
    Denied(&'static str),
}

impl Response {
    pub fn to_line(&self) -> String {
        match self {
            Self::Registered => "ok registered\n".to_owned(),
            Self::Cancelled => "ok cancelled\n".to_owned(),
            Self::Inspected(all) => format!(
                "ok {}\n",
                all.iter()
                    .map(|entry| format!("{}:{}", entry.launch_id, entry.pid))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Denied(reason) => format!("denied {reason}\n"),
        }
    }
}

/// Registrations by launch id, bounded.
#[derive(Debug, Default)]
pub struct Registry {
    owner_uid: u32,
    entries: BTreeMap<String, Registration>,
}

fn valid_launch_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_LAUNCH_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

impl Registry {
    pub fn new(owner_uid: u32) -> Self {
        Self {
            owner_uid,
            entries: BTreeMap::new(),
        }
    }

    pub fn get(&self, launch_id: &str) -> Option<&Registration> {
        self.entries.get(launch_id)
    }

    /// Handles one request line from an authenticated peer.
    pub fn handle(&mut self, peer: PeerCredentials, line: &str) -> Response {
        if peer.uid != self.owner_uid {
            return Response::Denied("wrong_user");
        }
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next(), words.next()) {
            (Some("register"), Some(id), Some(pid), None) => {
                let Ok(pid) = pid.parse::<u32>() else {
                    return Response::Denied("bad_pid");
                };
                if !valid_launch_id(id) {
                    return Response::Denied("bad_launch_id");
                }
                // A peer may only register itself: the pid is what the kernel says it is.
                if pid != peer.pid {
                    return Response::Denied("pid_is_not_the_peer");
                }
                if self.entries.contains_key(id) {
                    return Response::Denied("already_registered");
                }
                if self.entries.len() >= MAX_REGISTRATIONS {
                    return Response::Denied("registry_full");
                }
                self.entries.insert(
                    id.to_owned(),
                    Registration {
                        launch_id: id.to_owned(),
                        pid,
                        uid: peer.uid,
                    },
                );
                Response::Registered
            }
            (Some("cancel"), Some(id), None, None) => match self.entries.get(id) {
                Some(entry) if entry.pid == peer.pid => {
                    self.entries.remove(id);
                    Response::Cancelled
                }
                Some(_) => Response::Denied("not_the_registrant"),
                None => Response::Denied("unknown_launch_id"),
            },
            (Some("inspect"), None, None, None) => {
                Response::Inspected(self.entries.values().cloned().collect())
            }
            _ => Response::Denied("unknown_verb"),
        }
    }
}

/// Serves one connection: authenticates the peer, reads one bounded line, answers once.
pub fn serve_connection(registry: &mut Registry, stream: UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let (pid, uid, _) = crate::syscall::unix_peer_credentials(&stream)?;
    let mut line = String::new();
    let read = BufReader::new((&stream).take_bounded())
        .read_line(&mut line)
        .unwrap_or(0);
    let response = if read == 0 || !line.ends_with('\n') {
        Response::Denied("bad_request")
    } else {
        registry.handle(PeerCredentials { pid, uid }, &line)
    };
    (&stream).write_all(response.to_line().as_bytes())
}

trait TakeBounded: io::Read + Sized {
    fn take_bounded(self) -> io::Take<Self> {
        io::Read::take(self, MAX_LINE_BYTES)
    }
}
impl<T: io::Read> TakeBounded for T {}

#[derive(Debug)]
pub enum BindError {
    Directory(ConfinementError),
    /// The path exists and is not a socket.
    NotASocket,
    /// A live service already listens there.
    InUse,
    Io(io::Error),
}

/// Binds the registration socket `name` inside the confined runtime directory `dir`. A stale
/// socket (nothing accepts on it) is replaced; any other kind of file, or a live listener, is
/// refused. The socket is made owner-only.
pub fn bind(dir: &Path, name: &str, owner_uid: u32) -> Result<UnixListener, BindError> {
    ConfinedDir::open(dir, owner_uid).map_err(BindError::Directory)?;
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(BindError::Directory(ConfinementError::BadName));
    }
    let path = dir.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if UnixStream::connect(&path).is_ok() {
                return Err(BindError::InUse);
            }
            std::fs::remove_file(&path).map_err(BindError::Io)?;
        }
        Ok(_) => return Err(BindError::NotASocket),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(BindError::Io(error)),
    }
    let listener = UnixListener::bind(&path).map_err(BindError::Io)?;
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .map_err(BindError::Io)?;
    Ok(listener)
}

/// Binds without connecting to, deleting or replacing an existing socket. A runtime observer
/// must never block at startup on an old listener's full accept queue.
pub fn bind_fresh(dir: &Path, name: &str, owner_uid: u32) -> Result<UnixListener, BindError> {
    ConfinedDir::open(dir, owner_uid).map_err(BindError::Directory)?;
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(BindError::Directory(ConfinementError::BadName));
    }
    let path = dir.join(name);
    let listener = UnixListener::bind(&path).map_err(BindError::Io)?;
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .map_err(BindError::Io)?;
    Ok(listener)
}
