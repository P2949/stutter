//! The virtual keyboard of the isolated session. A uinput device is a kernel-wide input device: unless udev attaches it
//! to a seat of its own, the running compositor of seat0 picks it up and delivers its keys to the focused window. So no
//! event is ever written through it until the device's own udev record says it is attached to the isolated seat:
//!
//! * `DeviceInjector::new` takes the udev record of the created device and refuses unless it carries `ID_SEAT=<the
//!   isolated seat>`, which must be a seat other than `seat0` (a device with no `ID_SEAT` is on `seat0` by default);
//! * it also takes the `IsolatedSeat` attestation, so it cannot be built for the live session at all;
//! * a device that fails the check is never written to (the caller destroys it).
//!
//! `UinputDevice::create` is the only part that touches the kernel: it opens the node it is given and issues the
//! documented ioctl sequence. It is never called by this build (no sink is wired and it needs a session of its own to run
//! in); the request numbers and every other part are checked here without a device.

use std::{io::Write, os::fd::OwnedFd, path::Path};

use super::{Key, encode_key, isolated_seat::IsolatedSeat, seat_helper::Injector};

/// A created kernel device, destroyed when dropped.
pub struct UinputDevice {
    fd: OwnedFd,
}

impl UinputDevice {
    /// Create a keyboard on `node` (normally `/dev/uinput`) that can press exactly the keys a route may use. Only for a
    /// session of its own: the seat attestation is required, and the device must still pass `DeviceInjector::new`
    /// before anything is written through it.
    pub fn create(node: &Path, seat: &IsolatedSeat) -> Result<Self, &'static str> {
        let codes: Vec<u16> = Key::ALL.iter().map(|key| key.code()).collect();
        let name = format!("stutter-route-{}", seat.display());
        crate::syscall::uinput_keyboard(node, &codes, &name).map(|fd| Self { fd })
    }
}

impl Write for UinputDevice {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut file = std::fs::File::from(self.fd.try_clone()?);
        file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        crate::syscall::uinput_destroy(&self.fd);
    }
}

/// The seat a udev record (`E:` property lines) puts a device on: `ID_SEAT=`, or `seat0` when none is set.
pub fn udev_seat(record: &str) -> String {
    record
        .lines()
        .find_map(|line| line.strip_prefix("E:ID_SEAT="))
        .map_or_else(|| "seat0".to_owned(), |seat| seat.trim().to_owned())
}

/// Writes key events to a device proven attached to the isolated seat.
pub struct DeviceInjector<W: Write> {
    writer: W,
}

impl<W: Write> DeviceInjector<W> {
    /// `record` is the device\'s own udev record; `isolated_seat_name` is the seat the isolated session was given. The
    /// device must be on that seat, and that seat must not be `seat0` (the seat of the live session).
    pub fn new(
        writer: W,
        _seat: &IsolatedSeat,
        record: &str,
        isolated_seat_name: &str,
    ) -> Result<Self, &'static str> {
        let on = udev_seat(record);
        if isolated_seat_name.is_empty() || isolated_seat_name == "seat0" {
            return Err("IsolatedSeatNameIsTheLiveSeat");
        }
        if on != isolated_seat_name {
            return Err("DeviceNotOnTheIsolatedSeat");
        }
        Ok(Self { writer })
    }
}

impl<W: Write> Injector for DeviceInjector<W> {
    fn inject(&mut self, key: Key, down: bool) -> Result<(), &'static str> {
        self.writer
            .write_all(&encode_key(key, down))
            .and_then(|()| self.writer.flush())
            .map_err(|_| "InjectionFailed")
    }
}

#[cfg(test)]
#[path = "uinput_injector_tests.rs"]
mod tests;
