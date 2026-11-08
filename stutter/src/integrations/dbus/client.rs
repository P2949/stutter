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
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
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
        stream.write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            if line.len() > 512 {
                return Err(DbusError::AuthRejected("overlong reply".into()));
            }
            stream.read_exact(&mut byte)?;
            line.push(byte[0]);
        }
        let reply = String::from_utf8_lossy(&line).trim().to_owned();
        if !reply.starts_with("OK ") && reply != "OK" {
            return Err(DbusError::AuthRejected(reply));
        }
        stream.write_all(b"BEGIN\r\n")?;
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

    fn read_message(&mut self) -> Result<Message, DbusError> {
        let mut fixed = [0u8; 16];
        self.stream.read_exact(&mut fixed)?;
        let total = message_length(&fixed)?;
        let mut bytes = fixed.to_vec();
        bytes.resize(total, 0);
        self.stream.read_exact(&mut bytes[16..])?;
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
        self.stream.write_all(&method_call(
            serial,
            destination,
            path,
            interface,
            member,
            signature,
            body,
        ))?;
        let deadline = Instant::now() + TIMEOUT;
        for _ in 0..MAX_SKIPPED {
            if Instant::now() > deadline {
                return Err(DbusError::TimedOut);
            }
            let message = self.read_message()?;
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
