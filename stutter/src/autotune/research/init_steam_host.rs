//! The production `InitHost`: prefix initialization over the same authenticated `SteamPort` the
//! measured attempts use, so its launch passes the same admission (safe window, GPU health, disk
//! reserve, dispatcher identity, tickets) and its settlement the same independent witness. It differs
//! from an attempt only in purpose: nothing is measured, a result or its absence is not evidence, and
//! the process simply ending is the expected outcome. The composition supplies the context for the
//! initialization launch (the attempt slot and dispatch context bound to the campaign, boot, tool and
//! prefix) through `ContextFor`; this module never builds identity itself.

use super::{
    AttemptSlot, ProtonSource,
    dispatch_context::DispatchContextV1,
    prefix_init_stage::{InitHost, InitProcess},
    steam_host::{GameState, SteamPort},
};

/// The attempt slot and dispatch context the composition binds to one arm's initialization launch.
pub type ContextFor =
    Box<dyn FnMut(ProtonSource) -> anyhow::Result<(AttemptSlot, DispatchContextV1)> + Send>;

pub struct SteamInitHost<P: SteamPort> {
    port: P,
    context_for: ContextFor,
}

impl<P: SteamPort> SteamInitHost<P> {
    pub fn new(port: P, context_for: ContextFor) -> Self {
        Self { port, context_for }
    }
}

impl<P: SteamPort + Send> InitHost for SteamInitHost<P> {
    fn launch(&mut self, arm: ProtonSource) -> Result<(), &'static str> {
        if self.port.health_blocked() {
            return Err("BlockedGpuStability");
        }
        if !self.port.safe_window() {
            return Err("SafeWindowClosed");
        }
        let (slot, context) = (self.context_for)(arm).map_err(|_| "InitContextUnavailable")?;
        self.port.launch(&slot, &context).map_err(|error| {
            // A typed admission refusal keeps its own code; anything else is a plain refusal.
            error
                .downcast_ref::<super::driver_failure::LaunchFailure>()
                .map_or("InitLaunchRefused", |failure| failure.code)
        })
    }

    fn process(&mut self) -> Result<InitProcess, &'static str> {
        match self.port.state() {
            GameState::Starting | GameState::Running => Ok(InitProcess::Running),
            // The game ending is how initialization finishes; whether it produced a native result is
            // irrelevant here. Settlement is still proven by the port's independent witness.
            GameState::ExitedWithoutResult
            | GameState::Result { .. }
            | GameState::InvalidNativeResult => self
                .port
                .stop()
                .map(|()| InitProcess::Settled)
                .map_err(|_| "SettlementUnproven"),
            // Anything that voids the launch is settled before it is reported, then the stage fails.
            GameState::DesktopInterrupted => self.terminate().map(|()| InitProcess::Settled),
            GameState::GpuReset | GameState::IdentityMismatch => {
                self.terminate()?;
                Err("InitLaunchVoided")
            }
        }
    }

    fn terminate(&mut self) -> Result<(), &'static str> {
        self.port.stop().map_err(|_| "SettlementUnproven")
    }
}

#[cfg(test)]
#[path = "init_steam_host_tests.rs"]
mod tests;
