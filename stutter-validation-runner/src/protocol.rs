//! Length-prefixed JSON framing: a 4-byte big-endian length, then that many bytes of JSON.
//! Frames are size-bounded so a hostile peer cannot make the root service allocate freely.
use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};

/// Largest accepted frame (requests are tiny; 64 KiB is generous).
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame length {length} outside 1..={MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
