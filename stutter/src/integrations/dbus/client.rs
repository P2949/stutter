use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::wire::{
    Message, MessageType, Reader, Value, WireError, Writer, message_length, method_call,
    parse_message,
};

const TIMEOUT: Duration = Duration::from_secs(2);
/// Most messages skipped (signals, unrelated replies) while waiting for one reply.
const MAX_SKIPPED: usize = 64;

fn remaining(deadline: Instant) -> Result<Duration, DbusError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(DbusError::TimedOut)
}

fn read_exact_until(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), DbusError> {
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(bytes) {
            Ok(0) => {
                return Err(DbusError::Io(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )));
            }
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
        remaining(deadline)?;
    }
    Ok(())
}

fn write_all_until(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), DbusError> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(DbusError::Io(std::io::Error::from(
                    std::io::ErrorKind::WriteZero,
                )));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
        remaining(deadline)?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum DbusError {
    Io(std::io::Error),
    Wire(WireError),
    /// The server did not accept EXTERNAL authentication.
    AuthRejected(String),
    /// The bus answered with a D-Bus error.
    Remote(String),
    /// The reply had an unexpected shape.
    UnexpectedReply,
    TimedOut,
    NoBusAddress,
}

impl From<std::io::Error> for DbusError {
    fn from(error: std::io::Error) -> Self {
        if matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ) {
            Self::TimedOut
        } else {
            Self::Io(error)
        }
    }
}

impl From<WireError> for DbusError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusKind {
    System,
    Session,
}

pub fn system_bus_path() -> PathBuf {
    PathBuf::from("/run/dbus/system_bus_socket")
}

/// The session bus socket from `DBUS_SESSION_BUS_ADDRESS` (`unix:path=...`), when set.
pub fn session_bus_path() -> Option<PathBuf> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    address
        .split(',')
        .next()?
        .strip_prefix("unix:path=")
        .map(PathBuf::from)
}

pub struct DbusClient {
    stream: UnixStream,
    next_serial: u32,
    pub unique_name: String,
}

impl DbusClient {
    pub fn connect(kind: BusKind) -> Result<Self, DbusError> {
        let path = match kind {
            BusKind::System => system_bus_path(),
            BusKind::Session => session_bus_path().ok_or(DbusError::NoBusAddress)?,
        };
        Self::connect_path(&path, crate::syscall::geteuid())
    }

    /// Connects to the bus socket at `path`, authenticates as `uid`, and says hello.
    pub fn connect_path(path: &Path, uid: u32) -> Result<Self, DbusError> {
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        let address = socket2::SockAddr::unix(path)?;
        socket.connect_timeout(&address, TIMEOUT)?;
        let stream = UnixStream::from(socket);
        Self::handshake(stream, uid)
    }

    pub fn from_stream(stream: UnixStream, uid: u32) -> Result<Self, DbusError> {
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Self::handshake(stream, uid)
    }

    fn handshake(mut stream: UnixStream, uid: u32) -> Result<Self, DbusError> {
        let hex: String = uid
            .to_string()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let deadline = Instant::now() + TIMEOUT;
        write_all_until(
            &mut stream,
            format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes(),
            deadline,
        )?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            if line.len() > 512 {
                return Err(DbusError::AuthRejected("overlong reply".into()));
            }
            read_exact_until(&mut stream, &mut byte, deadline)?;
            line.push(byte[0]);
        }
        let reply = String::from_utf8_lossy(&line).trim().to_owned();
        if !reply.starts_with("OK ") && reply != "OK" {
            return Err(DbusError::AuthRejected(reply));
        }
        write_all_until(&mut stream, b"BEGIN\r\n", deadline)?;
        let mut client = Self {
            stream,
            next_serial: 1,
            unique_name: String::new(),
        };
        let reply = client.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            "",
            &[],
        )?;
        let Some(Value::Str(name)) = reply.first() else {
            return Err(DbusError::UnexpectedReply);
        };
        client.unique_name = name.clone();
        Ok(client)
    }

    fn read_message(&mut self, deadline: Instant) -> Result<Message, DbusError> {
        let mut fixed = [0u8; 16];
        read_exact_until(&mut self.stream, &mut fixed, deadline)?;
        let total = message_length(&fixed)?;
        let mut bytes = fixed.to_vec();
        bytes.resize(total, 0);
        read_exact_until(&mut self.stream, &mut bytes[16..], deadline)?;
        Ok(parse_message(&bytes)?)
    }

    /// Calls a method and returns the body values of the reply, or the remote error.
    pub fn call(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<Vec<Value>, DbusError> {
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        let deadline = Instant::now() + TIMEOUT;
        write_all_until(
            &mut self.stream,
            &method_call(
                serial,
                destination,
                path,
                interface,
                member,
                signature,
                body,
            ),
            deadline,
        )?;
        for _ in 0..MAX_SKIPPED {
            if Instant::now() > deadline {
                return Err(DbusError::TimedOut);
            }
            let message = self.read_message(deadline)?;
            if message.reply_serial != Some(serial) {
                continue; // a signal or someone else's reply
            }
            return match message.kind {
                MessageType::MethodReturn => {
                    let mut reader = Reader::new(&message.body);
                    let mut values = Vec::new();
                    let mut rest = message.signature.as_str();
                    while !rest.is_empty() {
                        let one = complete_type(rest)?;
                        values.push(reader.value(one)?);
                        rest = &rest[one.len()..];
                    }
                    Ok(values)
                }
                MessageType::Error => {
                    Err(DbusError::Remote(message.error_name.unwrap_or_default()))
                }
                _ => Err(DbusError::UnexpectedReply),
            };
        }
        Err(DbusError::TimedOut)
    }

    /// `org.freedesktop.DBus.GetNameOwner`: the unique name that owns `name`, `None` if nobody does.
    pub fn name_owner(&mut self, name: &str) -> Result<Option<String>, DbusError> {
        let mut body = Writer::default();
        body.string(name);
        match self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            "s",
            &body.buf,
        ) {
            Ok(values) => match values.first() {
                Some(Value::Str(owner)) => Ok(Some(owner.clone())),
                _ => Err(DbusError::UnexpectedReply),
            },
            Err(DbusError::Remote(error))
                if error == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
            {
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }

    /// `org.freedesktop.DBus.Properties.Get`: the value inside the returned variant.
    pub fn get_property(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> Result<Value, DbusError> {
        let mut body = Writer::default();
        body.string(interface);
        body.string(property);
        let values = self.call(
            destination,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            "ss",
            &body.buf,
        )?;
        match values.into_iter().next() {
            Some(Value::Variant(inner)) => Ok(*inner),
            _ => Err(DbusError::UnexpectedReply),
        }
    }
}

/// The first complete type of a reply signature (multi-value bodies).
fn complete_type(signature: &str) -> Result<&str, WireError> {
    let bytes = signature.as_bytes();
    let first = *bytes.first().ok_or(WireError::BadSignature)?;
    let length = match first {
        b'a' => 1 + complete_type(&signature[1..])?.len(),
        b'(' | b'{' => {
            let close = if first == b'(' { b')' } else { b'}' };
            let mut at = 1;
            while bytes.get(at) != Some(&close) {
                at += complete_type(signature.get(at..).ok_or(WireError::BadSignature)?)?.len();
            }
            at + 1
        }
        _ => 1,
    };
    signature.get(..length).ok_or(WireError::BadSignature)
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn timed_unix_connection_completes_the_existing_dbus_handshake() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bus");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::integrations::dbus::fake::serve(stream, |_| {
                crate::integrations::dbus::fake::Reply::Error("org.test.NotImplemented")
            });
        });
        let client = DbusClient::connect_path(&path, crate::syscall::geteuid()).unwrap();
        assert_eq!(client.unique_name, ":1.99");
        server.join().unwrap();
    }

    #[test]
    fn slow_incremental_sender_cannot_extend_total_read_deadline() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let sender = std::thread::spawn(move || {
            for _ in 0..10 {
                if writer.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        let mut data = [0u8; 10];
        assert!(matches!(
            read_exact_until(&mut reader, &mut data, start + Duration::from_millis(65)),
            Err(DbusError::TimedOut)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(reader);
        sender.join().unwrap();
    }

    #[test]
    fn blocked_writer_cannot_extend_total_write_deadline() {
        let (mut writer, _reader) = UnixStream::pair().unwrap();
        let start = Instant::now();
        let data = vec![0u8; 4 * 1024 * 1024];
        assert!(matches!(
            write_all_until(&mut writer, &data, start + Duration::from_millis(65)),
            Err(DbusError::TimedOut)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
