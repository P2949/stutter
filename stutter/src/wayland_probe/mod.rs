#[cfg(feature = "wayland-probe")]
mod ffi;
#[cfg(feature = "wayland-probe")]
mod memfd;
#[cfg(feature = "wayland-probe")]
mod protocol;
#[cfg(feature = "wayland-probe")]
mod snapshot;

use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone)]
pub struct WaylandProbeCommandInput {
    pub duration: Duration,
    pub output: Option<String>,
    pub fullscreen: bool,
    pub out_dir: PathBuf,
}

pub fn run_wayland_probe_command(input: WaylandProbeCommandInput) -> anyhow::Result<()> {
    imp::run(input)
}

#[cfg(not(feature = "wayland-probe"))]
mod imp {
    use super::*;

    pub(super) fn run(input: WaylandProbeCommandInput) -> anyhow::Result<()> {
        let WaylandProbeCommandInput {
            duration,
            output,
            fullscreen,
            out_dir,
        } = input;
        let _ = (duration, output, fullscreen, out_dir);
        anyhow::bail!(
            "wayland-probe command requires building stutter with --features wayland-probe"
        );
    }
}

#[cfg(feature = "wayland-probe")]
mod imp {
    use std::io::Write;

    use anyhow::{Context, Result};
    use wayland_client::Connection;

    use super::{WaylandProbeCommandInput, snapshot::State};

    pub(super) fn run(input: WaylandProbeCommandInput) -> Result<()> {
        let (mut state, events_path) = State::new(input)?;

        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        event_queue.roundtrip(&mut state)?;
        state.validate_globals()?;

        while state.running && state.started.elapsed() < state.duration {
            event_queue.blocking_dispatch(&mut state)?;
        }
        state.writer.flush()?;

        println!(
            "wrote {} Wayland presentation self-test events: {}",
            state.event_count,
            events_path.display()
        );
        Ok(())
    }
}
