//! An in-process fake bus for tests: real EXTERNAL authentication and Hello, then every call is
//! answered by a closure. It records the calls it saw so a test can prove what was (not) sent.
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    thread,
};

use super::wire::{Reader, Value, Writer, message_length};

#[derive(Clone, Debug)]
pub struct Call {
    pub serial: u32,
    pub destination: String,
    pub path: String,
    pub interface: String,
    pub member: String,
    pub signature: String,
    pub body: Vec<u8>,
}

pub enum Reply {
    /// A method return with this body signature and marshalled body.
    Body(String, Vec<u8>),
    Error(&'static str),
}

pub type Calls = Arc<Mutex<Vec<Call>>>;

fn read_call(stream: &mut UnixStream) -> std::io::Result<Call> {
    let mut fixed = [0u8; 16];
    stream.read_exact(&mut fixed)?;
    let total = message_length(&fixed).map_err(|_| std::io::Error::other("bad message"))?;
    let mut bytes = fixed.to_vec();
    bytes.resize(total, 0);
    stream.read_exact(&mut bytes[16..])?;
    let fields_len = u32::from_le_bytes([fixed[12], fixed[13], fixed[14], fixed[15]]) as usize;
    let mut reader = Reader::new(&bytes[..16 + fields_len]);
    reader.pos = 16;
    let mut call = Call {
        serial: u32::from_le_bytes([fixed[8], fixed[9], fixed[10], fixed[11]]),
        destination: String::new(),
        path: String::new(),
        interface: String::new(),
        member: String::new(),
        signature: String::new(),
        body: Vec::new(),
    };
    while reader.pos < 16 + fields_len {
        let aligned = reader.pos.next_multiple_of(8);
        if aligned >= 16 + fields_len {
            break;
        }
        reader.pos = aligned;
        let code = reader.u8().map_err(|_| std::io::Error::other("field"))?;
        let sig = reader
            .signature()
            .map_err(|_| std::io::Error::other("sig"))?;
        let value = reader
            .value(&sig)
            .map_err(|_| std::io::Error::other("value"))?;
        match (code, value) {
            (1, Value::Path(text)) => call.path = text,
            (2, Value::Str(text)) => call.interface = text,
            (3, Value::Str(text)) => call.member = text,
            (6, Value::Str(text)) => call.destination = text,
            (8, Value::Signature(text)) => call.signature = text,
            _ => {}
        }
    }
    call.body = bytes[16 + fields_len.next_multiple_of(8)..].to_vec();
    Ok(call)
}

fn header(
    kind: u8,
    reply_serial: u32,
    signature: &str,
    error: Option<&str>,
    body: &[u8],
) -> Vec<u8> {
    let mut out = Writer::default();
    out.u8(b'l');
    out.u8(kind);
    out.u8(0);
    out.u8(1);
    out.u32(body.len() as u32);
    out.u32(1);
    let length_at = out.buf.len();
    out.u32(0);
    let start = out.buf.len();
    if let Some(name) = error {
        out.align(8);
        out.u8(4);
        out.signature("s");
        out.string(name);
    }
    out.align(8);
    out.u8(5);
    out.signature("u");
    out.u32(reply_serial);
    if !signature.is_empty() {
        out.align(8);
        out.u8(8);
        out.signature("g");
        out.signature(signature);
    }
    let fields_len = out.buf.len() - start;
    out.buf[length_at..length_at + 4].copy_from_slice(&(fields_len as u32).to_le_bytes());
    out.align(8);
    out.buf.extend_from_slice(body);
    out.buf
}

/// Starts the fake on `server_end`; the returned handle lists every non-Hello call received.
pub fn serve(mut stream: UnixStream, answer: impl Fn(&Call) -> Reply + Send + 'static) -> Calls {
    let calls: Calls = Arc::default();
    let seen = Arc::clone(&calls);
    thread::spawn(move || {
        let mut nul = [0u8; 1];
        if stream.read_exact(&mut nul).is_err() {
            return;
        }
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            if stream.read_exact(&mut byte).is_err() {
                return;
            }
            line.push(byte[0]);
        }
        let _ = stream.write_all(b"OK 0123456789abcdef\r\n");
        let mut begin = [0u8; 7];
        if stream.read_exact(&mut begin).is_err() {
            return;
        }
        while let Ok(call) = read_call(&mut stream) {
            let bytes = if call.member == "Hello" {
                let mut body = Writer::default();
                body.string(":1.99");
                header(2, call.serial, "s", None, &body.buf)
            } else {
                if let Ok(mut seen) = seen.lock() {
                    seen.push(call.clone());
                }
                match answer(&call) {
                    Reply::Body(signature, body) => header(2, call.serial, &signature, None, &body),
                    Reply::Error(name) => header(3, call.serial, "", Some(name), &[]),
                }
            };
            if stream.write_all(&bytes).is_err() {
                return;
            }
        }
    });
    calls
}
