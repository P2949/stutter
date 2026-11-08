use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    thread,
};

use super::{DbusClient, DbusError, Value, wire::*};

/// A one-connection fake bus: EXTERNAL auth, Hello, then a canned answer per member.
fn fake_bus(
    mut stream: UnixStream,
    answer: impl Fn(&str, &str) -> Option<Vec<u8>> + Send + 'static,
) {
    thread::spawn(move || {
        let mut nul = [0u8; 1];
        stream.read_exact(&mut nul).unwrap();
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            stream.read_exact(&mut byte).unwrap();
            line.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&line).starts_with("AUTH EXTERNAL "));
        stream.write_all(b"OK 0123456789abcdef\r\n").unwrap();
        let mut begin = [0u8; 7];
        stream.read_exact(&mut begin).unwrap();
        assert_eq!(&begin, b"BEGIN\r\n");
        while let Ok(call) = read_call(&mut stream) {
            let reply = match call.member.as_str() {
                "Hello" => Some(reply_string(call.serial, ":1.99")),
                _ => answer(&call.member, &call.interface)
                    .map(|body| raw_reply(call.serial, &call.reply_signature, &body)),
            };
            let bytes = reply.unwrap_or_else(|| {
                error_reply(call.serial, "org.freedesktop.DBus.Error.NameHasNoOwner")
            });
            stream.write_all(&bytes).unwrap();
        }
    });
}

struct Call {
    serial: u32,
    member: String,
    interface: String,
    reply_signature: String,
}

fn read_call(stream: &mut UnixStream) -> std::io::Result<Call> {
    let mut fixed = [0u8; 16];
    stream.read_exact(&mut fixed)?;
    let total = message_length(&fixed).unwrap();
    let mut bytes = fixed.to_vec();
    bytes.resize(total, 0);
    stream.read_exact(&mut bytes[16..])?;
    // Pull the member and interface out of the header fields by parsing them as a reader would.
    let fields_len = u32::from_le_bytes([fixed[12], fixed[13], fixed[14], fixed[15]]) as usize;
    let mut reader = Reader::new(&bytes[..16 + fields_len]);
    reader.pos = 16;
    let (mut member, mut interface) = (String::new(), String::new());
    while reader.pos < 16 + fields_len {
        let aligned = reader.pos.next_multiple_of(8);
        if aligned >= 16 + fields_len {
            break;
        }
        reader.pos = aligned;
        let code = reader.u8().unwrap();
        let sig = reader.signature().unwrap();
        let value = reader.value(&sig).unwrap();
        match (code, value) {
            (3, Value::Str(text)) => member = text,
            (2, Value::Str(text)) => interface = text,
            _ => {}
        }
    }
    Ok(Call {
        serial: u32::from_le_bytes([fixed[8], fixed[9], fixed[10], fixed[11]]),
        member: member.clone(),
        interface,
        reply_signature: if member == "Get" {
            "v".into()
        } else {
            "s".into()
        },
    })
}

fn header(
    kind: u8,
    serial: u32,
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
    out.u32(serial);
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

fn reply_string(reply_serial: u32, text: &str) -> Vec<u8> {
    let mut body = Writer::default();
    body.string(text);
    header(2, 1, reply_serial, "s", None, &body.buf)
}

fn raw_reply(reply_serial: u32, signature: &str, body: &[u8]) -> Vec<u8> {
    header(2, 2, reply_serial, signature, None, body)
}

fn error_reply(reply_serial: u32, name: &str) -> Vec<u8> {
    header(3, 3, reply_serial, "", Some(name), &[])
}

fn variant_i32(value: i32) -> Vec<u8> {
    let mut body = Writer::default();
    body.signature("i");
    body.align(4);
    body.buf.extend(value.to_le_bytes());
    body.buf
}

#[test]
fn values_round_trip_through_the_wire_format() {
    let mut writer = Writer::default();
    writer.u8(7);
    writer.u32(300);
    writer.string("héllo");
    writer.signature("a{sv}");
    let mut reader = Reader::new(&writer.buf);
    assert_eq!(reader.value("y").unwrap(), Value::Byte(7));
    assert_eq!(reader.value("u").unwrap(), Value::U32(300));
    assert_eq!(reader.value("s").unwrap(), Value::Str("héllo".into()));
    assert_eq!(reader.value("g").unwrap(), Value::Signature("a{sv}".into()));
}

#[test]
fn arrays_dicts_variants_and_structs_decode_with_their_alignment() {
    // a{sv} with one entry: "k" -> variant(u32 5), then a struct (is).
    let mut body = Writer::default();
    let length_at = body.buf.len();
    body.u32(0);
    body.align(8);
    let start = body.buf.len();
    body.string("k");
    body.signature("u");
    body.u32(5);
    let length = (body.buf.len() - start) as u32;
    body.buf[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
    body.align(8);
    body.buf.extend(9i32.to_le_bytes());
    body.string("nine");
    let mut reader = Reader::new(&body.buf);
    assert_eq!(
        reader.value("a{sv}").unwrap(),
        Value::Dict(vec![(
            Value::Str("k".into()),
            Value::Variant(Box::new(Value::U32(5)))
        )])
    );
    assert_eq!(
        reader.value("(is)").unwrap(),
        Value::Struct(vec![Value::I32(9), Value::Str("nine".into())])
    );
}

#[test]
fn big_endian_oversize_truncated_and_malformed_messages_are_errors_not_panics() {
    let mut big_endian = [0u8; 16];
    big_endian[0] = b'B';
    assert_eq!(message_length(&big_endian), Err(WireError::BigEndian));
    let mut huge = [0u8; 16];
    huge[0] = b'l';
    huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(message_length(&huge), Err(WireError::TooLarge));
    let good = method_call(1, "d", "/p", "i.f", "M", "", &[]);
    assert!(parse_message(&good).is_ok());
    assert!(parse_message(&good[..good.len() - 3]).is_err());
    let mut seed = 0x9E37_79B9u64;
    for round in 0..5000 {
        let mut noisy = good.clone();
        for _ in 0..3 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed % noisy.len() as u64) as usize;
            noisy[at] = (seed >> 24) as u8;
        }
        let _ = parse_message(&noisy);
        let mut reader = Reader::new(&noisy);
        let _ = reader.value(["a{sv}", "(is)", "v", "aay", "s", "a(ssa{sv})"][round % 6]);
    }
    let mut deep = Reader::new(&[0u8; 64]);
    assert_eq!(deep.value(&"a".repeat(40)).unwrap_err(), WireError::TooDeep);
}

#[test]
fn a_client_authenticates_says_hello_and_reads_properties_and_name_owners() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    fake_bus(server_end, |member, interface| match (member, interface) {
        ("Get", _) => Some(variant_i32(2)),
        ("GetNameOwner", _) => Some({
            let mut body = Writer::default();
            body.string(":1.7");
            body.buf
        }),
        _ => None,
    });
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    assert_eq!(client.unique_name, ":1.99");
    let property = client
        .get_property(
            "com.feralinteractive.GameMode",
            "/com/feralinteractive/GameMode",
            "com.feralinteractive.GameMode",
            "ClientCount",
        )
        .unwrap();
    assert_eq!(property, Value::I32(2));
    assert_eq!(
        client.name_owner("some.Name").unwrap().as_deref(),
        Some(":1.7")
    );
}

#[test]
fn a_name_with_no_owner_is_none_and_other_remote_errors_are_errors() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    fake_bus(server_end, |_, _| None);
    let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
    assert_eq!(client.name_owner("absent.Name").unwrap(), None);
    assert!(matches!(
        client.get_property("a.b", "/a", "a.b", "P"),
        Err(DbusError::Remote(name)) if name.contains("NameHasNoOwner")
    ));
}

#[test]
fn a_rejected_authentication_is_reported_and_the_real_system_bus_answers_read_only_calls() {
    let (client_end, mut server_end) = UnixStream::pair().unwrap();
    thread::spawn(move || {
        let mut sink = [0u8; 64];
        let _ = server_end.read(&mut sink);
        let _ = server_end.write_all(b"REJECTED EXTERNAL\r\n");
    });
    assert!(matches!(
        DbusClient::from_stream(client_end, 1000),
        Err(DbusError::AuthRejected(_))
    ));
    // Live, read-only: the bus daemon itself always owns its own name.
    if super::system_bus_path().exists()
        && let Ok(mut client) = DbusClient::connect(super::BusKind::System)
    {
        assert!(client.unique_name.starts_with(':'));
        assert!(client.name_owner("org.freedesktop.DBus").unwrap().is_some());
        assert_eq!(client.name_owner("no.such.Name.stutter").unwrap(), None);
    }
}
