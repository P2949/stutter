//! Linux boot identity used to keep durable task state away from reused PIDs after reboot.
use std::io;

pub fn current_boot_id() -> io::Result<String> {
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let value = value.trim();
    if !valid_boot_id(value) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Linux boot identity",
        ));
    }
    Ok(value.to_owned())
}

pub fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
