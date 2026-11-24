//! The helper that runs inside the isolated session and turns the route's encoded events into key presses. This is its
//! whole logic, with the device behind a trait: the code that registers a virtual keyboard is the one thing not in this
//! build (it would be picked up by the running compositor of whichever session it is registered in). What is here and
//! tested:
//!
//! * it is built only from an attested `IsolatedSeat`, so it cannot be constructed for the live session;
//! * it reads the stream record pair by record pair and injects only what `decode` accepts (a key press or release of a
//!   key a route may use); anything else stops it;
//! * it tracks every key it holds down, and on end of stream, a refused event or a failed injection it releases every one
//!   of them, so the session never ends with a key stuck down, and it says whether every release was accepted;
//! * it acknowledges each key transition it injected, so the sender knows a write was delivered and not merely read.

use std::{
    collections::BTreeSet,
    io::{Read, Write},
};

use super::{
    EVENT_BYTES, Key,
    isolated_seat::IsolatedSeat,
    seat_socket_sink::{ACK, decode},
};

/// Whatever turns a key transition into a real key event in the isolated session.
pub trait Injector {
    fn inject(&mut self, key: Key, down: bool) -> Result<(), &'static str>;
}

/// What a finished serve says.
#[derive(Debug, PartialEq, Eq)]
pub struct Served {
    /// Key transitions injected.
    pub injected: usize,
    /// Why it stopped early (`None`: the stream ended cleanly).
    pub stopped: Option<&'static str>,
    /// Every key it had down was released and the release accepted.
    pub all_released: bool,
}

pub struct SeatHelper<I: Injector> {
    injector: I,
    held: BTreeSet<u16>,
    seat: String,
}

impl<I: Injector> SeatHelper<I> {
    /// Only for a seat proven to be the game\'s own.
    pub fn new(injector: I, seat: &IsolatedSeat) -> Self {
        Self {
            injector,
            held: BTreeSet::new(),
            seat: seat.display().to_owned(),
        }
    }

    /// The display this helper serves.
    pub fn display(&self) -> &str {
        &self.seat
    }

    fn release_all(&mut self) -> bool {
        let mut clean = true;
        for code in std::mem::take(&mut self.held) {
            if let Some(key) = Key::ALL.into_iter().find(|key| key.code() == code)
                && self.injector.inject(key, false).is_err()
            {
                // A release that was not accepted stays owed.
                self.held.insert(code);
                clean = false;
            }
        }
        clean
    }

    /// Serve one connection until its end, injecting each decoded transition and acknowledging it on `acks`.
    pub fn serve(&mut self, stream: &mut impl Read, acks: &mut impl Write) -> Served {
        let mut injected = 0usize;
        let mut stopped = None;
        let mut record = [0u8; 2 * EVENT_BYTES];
        'stream: loop {
            let mut have = 0usize;
            while have < record.len() {
                match stream.read(&mut record[have..]) {
                    Ok(0) => break,
                    Ok(read) => have += read,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        stopped = Some("SinkStreamBroken");
                        break 'stream;
                    }
                }
            }
            // The stream ended between records: clean. In the middle of one: torn, and not a clean end.
            if have == 0 {
                break;
            }
            if have < record.len() {
                stopped = Some("SinkStreamTorn");
                break;
            }
            let Ok(transitions) = decode(&record) else {
                stopped = Some("SinkEventRefused");
                break 'stream;
            };
            for (key, down) in transitions {
                if self.injector.inject(key, down).is_err() {
                    stopped = Some("InjectionFailed");
                    break 'stream;
                }
                if down {
                    self.held.insert(key.code());
                } else {
                    self.held.remove(&key.code());
                }
                injected += 1;
                // Acknowledged only after the transition was injected.
                if acks.write_all(&[ACK]).and_then(|()| acks.flush()).is_err() {
                    stopped = Some("SinkAckFailed");
                    break 'stream;
                }
            }
        }
        let all_released = self.release_all();
        Served {
            injected,
            stopped,
            all_released,
        }
    }
}

#[cfg(test)]
#[path = "seat_helper_tests.rs"]
mod tests;
