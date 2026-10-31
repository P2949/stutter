//! Process handles preserve the ownership difference between a child and a systemd actor.
use std::time::{Duration, Instant};

use super::world::{OwnedChild, signal};

pub(super) enum ControllerProcess {
    Direct(OwnedChild),
    Service(Box<super::service::Process>),
}

#[derive(Clone, Copy)]
pub(super) struct ControllerExit {
    code: Option<i32>,
    signal: Option<i32>,
}
impl ControllerExit {
    pub(super) fn code(self) -> Option<i32> {
        self.code
    }
    pub(super) fn signal(self) -> Option<i32> {
        self.signal
    }
}
impl ControllerProcess {
    pub(super) fn pid(&self) -> u32 {
        match self {
            Self::Direct(child) => child.pid(),
            Self::Service(process) => process.pid,
        }
    }
    pub(super) fn try_wait(&mut self) -> anyhow::Result<Option<ControllerExit>> {
        use std::os::unix::process::ExitStatusExt;
        Ok(match self {
            Self::Direct(child) => child.0.try_wait()?.map(|status| ControllerExit {
                code: status.code(),
                signal: status.signal(),
            }),
            // A systemd main process may already have been replaced. Preserve unknown exit status
            // instead of inventing a success code; runtime boundary receipts retain unit results.
            Self::Service(process) => (!process.alive()).then_some(ControllerExit {
                code: None,
                signal: None,
            }),
        })
    }
    pub(super) fn signal(&mut self, value: i32) -> anyhow::Result<()> {
        match self {
            Self::Direct(child) => signal(child.pid(), value),
            Self::Service(process) => process.signal(value)?,
        }
        Ok(())
    }
}

pub(super) enum Contention {
    Direct { _guard: OwnedChild },
    Service { _guard: super::service::Contention },
}

pub(super) fn wait_controller(
    child: &mut ControllerProcess,
    timeout: Duration,
) -> anyhow::Result<Option<ControllerExit>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
