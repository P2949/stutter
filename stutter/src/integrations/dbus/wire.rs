//! The D-Bus wire format, little-endian only, with every length bounded.
//!
//! Just enough to call a method and read its reply: marshalling of the basic types, arrays,
//! structs, dictionary entries and variants, message headers and reply parsing. Nothing here opens
//! a socket. A message that is big-endian, oversize, truncated or malformed is an error, never a
//! panic and never an unbounded read.
use std::fmt;

pub const MAX_FIELDS_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Signature(String),
    Variant(Box<Value>),
    Array(Vec<Value>),
    Struct(Vec<Value>),
    Dict(Vec<(Value, Value)>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    BigEndian,
    TooLarge,
    BadSignature,
    BadString,
    TooDeep,
    BadMessage,
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn align(&mut self, to: usize) {
        while !self.buf.len().is_multiple_of(to) {
            self.buf.push(0);
        }
    }

    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn u32(&mut self, value: u32) {
        self.align(4);
        self.buf.extend(value.to_le_bytes());
    }

    pub fn string(&mut self, value: &str) {
        self.u32(value.len() as u32);
        self.buf.extend(value.as_bytes());
        self.buf.push(0);
    }

    pub fn signature(&mut self, value: &str) {
        self.buf.push(value.len() as u8);
        self.buf.extend(value.as_bytes());
        self.buf.push(0);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn align(&mut self, to: usize) -> Result<(), WireError> {
        self.pos = self
            .pos
            .checked_next_multiple_of(to)
            .ok_or(WireError::Truncated)?;
        if self.pos > self.bytes.len() {
            return Err(WireError::Truncated);
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.align(N.min(8))?;
        self.take(N)?.try_into().map_err(|_| WireError::Truncated)
    }

    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.fixed::<4>()?))
    }

    pub fn string(&mut self) -> Result<String, WireError> {
        let length = self.u32()? as usize;
        let raw = self.take(length)?;
        if self.u8()? != 0 {
            return Err(WireError::BadString);
        }
        String::from_utf8(raw.to_vec()).map_err(|_| WireError::BadString)
    }

    pub fn signature(&mut self) -> Result<String, WireError> {
        let length = usize::from(self.u8()?);
        let raw = self.take(length)?;
        if self.u8()? != 0 {
            return Err(WireError::BadString);
        }
        String::from_utf8(raw.to_vec()).map_err(|_| WireError::BadString)
    }

    /// One value of complete type `signature` (a single complete type).
    pub fn value(&mut self, signature: &str) -> Result<Value, WireError> {
        // The specification caps nesting at 32 arrays and 32 structs; refuse deeper types outright.
        let (mut depth, mut deepest) = (0usize, 0usize);
        for byte in signature.bytes() {
            match byte {
                b'a' | b'(' | b'{' => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                b')' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if deepest > 32 {
            return Err(WireError::TooDeep);
        }
        self.value_at(signature.as_bytes(), 0)
    }

    fn value_at(&mut self, signature: &[u8], depth: usize) -> Result<Value, WireError> {
        if depth > MAX_DEPTH {
            return Err(WireError::TooDeep);
        }
        let first = *signature.first().ok_or(WireError::BadSignature)?;
        Ok(match first {
            b'y' => Value::Byte(self.u8()?),
            b'b' => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(WireError::BadMessage),
            },
            b'n' => Value::I16(i16::from_le_bytes(self.fixed::<2>()?)),
            b'q' => Value::U16(u16::from_le_bytes(self.fixed::<2>()?)),
            b'i' => Value::I32(i32::from_le_bytes(self.fixed::<4>()?)),
            b'u' => Value::U32(self.u32()?),
            b'x' => Value::I64(i64::from_le_bytes(self.fixed::<8>()?)),
            b't' => Value::U64(u64::from_le_bytes(self.fixed::<8>()?)),
            b'd' => Value::F64(f64::from_le_bytes(self.fixed::<8>()?)),
            b's' => Value::Str(self.string()?),
            b'o' => Value::Path(self.string()?),
            b'g' => Value::Signature(self.signature()?),
            b'v' => {
                let inner = self.signature()?;
                Value::Variant(Box::new(self.value_at(inner.as_bytes(), depth + 1)?))
            }
            b'a' => {
                let element = &signature[1..];
                let length = self.u32()? as usize;
                if length > MAX_BODY_BYTES {
                    return Err(WireError::TooLarge);
                }
                // Elements of a struct or dict entry start 8-aligned even when the array is empty.
                if matches!(element.first(), Some(b'(' | b'{')) {
                    self.align(8)?;
                } else if let Some(&kind) = element.first() {
                    let alignment = match kind {
                        b'n' | b'q' => 2,
                        b'x' | b't' | b'd' => 8,
                        b'y' | b'g' | b'v' => 1,
                        _ => 4,
                    };
                    self.align(alignment)?;
                }
                let end = self.pos.checked_add(length).ok_or(WireError::Truncated)?;
                if end > self.bytes.len() {
                    return Err(WireError::Truncated);
                }
                if element.first() == Some(&b'{') {
                    let (key, value) = dict_types(element)?;
                    let mut entries = Vec::new();
                    while self.pos < end {
                        self.align(8)?;
                        entries.push((
                            self.value_at(key, depth + 1)?,
                            self.value_at(value, depth + 1)?,
                        ));
                    }
                    Value::Dict(entries)
                } else {
                    let mut items = Vec::new();
                    while self.pos < end {
                        items.push(self.value_at(single_type(element)?, depth + 1)?);
                    }
                    Value::Array(items)
                }
            }
            b'(' => {
                self.align(8)?;
                let mut fields = Vec::new();
                let mut rest = &signature[1..];
                while rest.first() != Some(&b')') {
                    let one = single_type(rest)?;
                    fields.push(self.value_at(one, depth + 1)?);
                    rest = &rest[one.len()..];
                }
                Value::Struct(fields)
            }
            _ => return Err(WireError::BadSignature),
        })
    }
}

/// The first complete type of `signature`.
fn single_type(signature: &[u8]) -> Result<&[u8], WireError> {
    let first = *signature.first().ok_or(WireError::BadSignature)?;
    let length = match first {
        b'a' => 1 + single_type(&signature[1..])?.len(),
        b'(' | b'{' => {
            let close = if first == b'(' { b')' } else { b'}' };
            let mut at = 1;
            while signature.get(at) != Some(&close) {
                at += single_type(signature.get(at..).ok_or(WireError::BadSignature)?)?.len();
            }
            at + 1
        }
        b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g'
        | b'v' => 1,
        _ => return Err(WireError::BadSignature),
    };
    signature.get(..length).ok_or(WireError::BadSignature)
}

fn dict_types(entry: &[u8]) -> Result<(&[u8], &[u8]), WireError> {
    let inner = &entry[1..];
    let key = single_type(inner)?;
    let value = single_type(&inner[key.len()..])?;
    Ok((key, value))
}

#[derive(Debug, PartialEq, Eq)]
pub enum MessageType {
    MethodCall,
    MethodReturn,
    Error,
    Signal,
}

#[derive(Debug)]
pub struct Message {
    pub kind: MessageType,
    pub reply_serial: Option<u32>,
    pub error_name: Option<String>,
    pub signature: String,
    pub body: Vec<u8>,
}

/// The total bytes of the message whose 16-byte fixed header is `fixed`, or an error.
pub fn message_length(fixed: &[u8; 16]) -> Result<usize, WireError> {
    if fixed[0] != b'l' {
        return Err(WireError::BigEndian);
    }
    let body = u32::from_le_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]) as usize;
    let fields = u32::from_le_bytes([fixed[12], fixed[13], fixed[14], fixed[15]]) as usize;
    if body > MAX_BODY_BYTES || fields > MAX_FIELDS_BYTES {
        return Err(WireError::TooLarge);
    }
    Ok(16 + fields.next_multiple_of(8) + body)
}

pub fn parse_message(bytes: &[u8]) -> Result<Message, WireError> {
    let fixed: [u8; 16] = bytes
        .get(..16)
        .ok_or(WireError::Truncated)?
        .try_into()
        .map_err(|_| WireError::Truncated)?;
    let total = message_length(&fixed)?;
    if bytes.len() < total {
        return Err(WireError::Truncated);
    }
    let kind = match fixed[1] {
        1 => MessageType::MethodCall,
        2 => MessageType::MethodReturn,
        3 => MessageType::Error,
        4 => MessageType::Signal,
        _ => return Err(WireError::BadMessage),
    };
    if fixed[3] != 1 {
        return Err(WireError::BadMessage);
    }
    let fields_len = u32::from_le_bytes([fixed[12], fixed[13], fixed[14], fixed[15]]) as usize;
    let body_start = 16 + fields_len.next_multiple_of(8);
    let mut reader = Reader::new(&bytes[..16 + fields_len]);
    reader.pos = 16;
    let (mut reply_serial, mut error_name, mut signature) = (None, None, String::new());
    while reader.pos < 16 + fields_len {
        reader.align(8)?;
        if reader.pos >= 16 + fields_len {
            break;
        }
        let code = reader.u8()?;
        let inner = reader.signature()?;
        let value = reader.value(&inner)?;
        match (code, value) {
            (4, Value::Str(name)) => error_name = Some(name),
            (5, Value::U32(serial)) => reply_serial = Some(serial),
            (8, Value::Signature(sig)) => signature = sig,
            _ => {}
        }
    }
    Ok(Message {
        kind,
        reply_serial,
        error_name,
        signature,
        body: bytes[body_start..total].to_vec(),
    })
}

/// A method-call message with a body already marshalled for `body_signature`.
pub fn method_call(
    serial: u32,
    destination: &str,
    path: &str,
    interface: &str,
    member: &str,
    body_signature: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut out = Writer::default();
    out.u8(b'l');
    out.u8(1); // method call
    out.u8(0); // flags
    out.u8(1); // protocol version
    out.u32(body.len() as u32);
    out.u32(serial);
    let length_at = out.buf.len();
    out.u32(0); // fields length, patched below
    let fields_start = out.buf.len();
    let field = |out: &mut Writer, code: u8, kind: u8, text: &str| {
        out.align(8);
        out.u8(code);
        out.signature(std::str::from_utf8(&[kind]).unwrap_or("s"));
        out.string(text);
    };
    field(&mut out, 1, b'o', path);
    if !interface.is_empty() {
        field(&mut out, 2, b's', interface);
    }
    field(&mut out, 3, b's', member);
    field(&mut out, 6, b's', destination);
    if !body_signature.is_empty() {
        out.align(8);
        out.u8(8);
        out.signature("g");
        out.signature(body_signature);
    }
    let fields_len = out.buf.len() - fields_start;
    out.buf[length_at..length_at + 4].copy_from_slice(&(fields_len as u32).to_le_bytes());
    out.align(8);
    out.buf.extend_from_slice(body);
    out.buf
}
