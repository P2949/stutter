use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use wayland_client::{
    QueueHandle, WEnum,
    protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::{
    wp::presentation_time::client::{wp_presentation, wp_presentation_feedback},
    xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};

use super::WaylandProbeCommandInput;
use crate::{
    artifacts::{ArtifactKind, artifact_path},
    recorder::WaylandPresentationEventRecord,
};

pub(super) const WIDTH: u32 = 320;
pub(super) const HEIGHT: u32 = 240;

pub(super) struct State {
    pub(super) running: bool,
    pub(super) duration: Duration,
    pub(super) started: Instant,
    pub(super) start_monotonic_ns: u64,
    pub(super) output_filter: Option<String>,
    pub(super) fullscreen: bool,
    pub(super) base_surface: Option<wl_surface::WlSurface>,
    pub(super) buffer: Option<wl_buffer::WlBuffer>,
    pub(super) shm_pool: Option<wl_shm_pool::WlShmPool>,
    pub(super) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(super) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(super) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(super) presentation: Option<wp_presentation::WpPresentation>,
    pub(super) configured: bool,
    pub(super) pending_feedback: bool,
    pub(super) frame_index: u64,
    pub(super) selected_output: Option<wl_output::WlOutput>,
    pub(super) writer: BufWriter<File>,
    pub(super) event_count: u64,
}

impl State {
    pub(super) fn new(input: WaylandProbeCommandInput) -> Result<(Self, std::path::PathBuf)> {
        fs::create_dir_all(&input.out_dir)
            .with_context(|| format!("failed to create {}", input.out_dir.display()))?;
        let events_path = artifact_path(&input.out_dir, ArtifactKind::WaylandPresentationEvents);
        let writer = BufWriter::new(
            File::create(&events_path)
                .with_context(|| format!("failed to create {}", events_path.display()))?,
        );

        let start_monotonic_ns = super::ffi::monotonic_now_ns();
        let state = Self {
            running: true,
            duration: input.duration,
            started: Instant::now(),
            start_monotonic_ns,
            output_filter: input.output,
            fullscreen: input.fullscreen,
            base_surface: None,
            buffer: None,
            shm_pool: None,
            wm_base: None,
            xdg_surface: None,
            toplevel: None,
            presentation: None,
            configured: false,
            pending_feedback: false,
            frame_index: 0,
            selected_output: None,
            writer,
            event_count: 0,
        };
        Ok((state, events_path))
    }

    pub(super) fn validate_globals(&self) -> Result<()> {
        if self.base_surface.is_none() {
            anyhow::bail!("Wayland compositor global wl_compositor is unavailable");
        }
        if self.buffer.is_none() {
            anyhow::bail!("Wayland wl_shm global is unavailable");
        }
        if self.wm_base.is_none() {
            anyhow::bail!("Wayland xdg_wm_base global is unavailable");
        }
        if self.presentation.is_none() {
            anyhow::bail!("Wayland wp_presentation global is unavailable");
        }
        Ok(())
    }

    pub(super) fn maybe_init_xdg_surface(&mut self, qh: &QueueHandle<State>) {
        if self.xdg_surface.is_some() {
            return;
        }
        let (Some(wm_base), Some(base_surface)) =
            (self.wm_base.as_ref(), self.base_surface.as_ref())
        else {
            return;
        };

        let xdg_surface = wm_base.get_xdg_surface(base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title("stutter wayland-probe".into());
        if self.fullscreen {
            toplevel.set_fullscreen(self.selected_output.as_ref());
        }
        base_surface.commit();

        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    pub(super) fn maybe_submit_frame(&mut self, qh: &QueueHandle<State>) {
        if self.pending_feedback || !self.configured {
            return;
        }
        let (Some(surface), Some(buffer), Some(presentation)) = (
            self.base_surface.as_ref(),
            self.buffer.as_ref(),
            self.presentation.as_ref(),
        ) else {
            return;
        };
        if self.started.elapsed() >= self.duration {
            self.running = false;
            return;
        }

        let commit_ns = super::ffi::monotonic_now_ns();
        presentation.feedback(
            surface,
            qh,
            super::protocol::FrameFeedbackData { commit_ns },
        );
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, WIDTH as i32, HEIGHT as i32);
        surface.commit();
        self.pending_feedback = true;
        self.frame_index = self.frame_index.saturating_add(1);
    }

    pub(super) fn write_presented(
        &mut self,
        commit_ns: u64,
        presented_ns: u64,
        refresh_ns: u32,
        sequence: u64,
        flags: WEnum<wp_presentation_feedback::Kind>,
    ) {
        let (zero_copy, flag_names) = presentation_flags(flags);
        let elapsed_ms = presented_ns
            .checked_sub(self.start_monotonic_ns)
            .map(|ns| ns / 1_000_000)
            .unwrap_or_else(|| self.started.elapsed().as_millis() as u64);
        let event = WaylandPresentationEventRecord {
            elapsed_ms,
            source: "self_test".to_owned(),
            app_id: Some("stutter".to_owned()),
            surface_role: Some("self_test".to_owned()),
            commit_ns: Some(commit_ns),
            presented_ns: Some(presented_ns),
            commit_to_present_ns: presented_ns.checked_sub(commit_ns),
            output_name: self.output_filter.clone(),
            refresh_ns: (refresh_ns != 0).then_some(refresh_ns as u64),
            sequence: Some(sequence),
            zero_copy,
            discarded: false,
            flags: flag_names,
            confidence: "high".to_owned(),
        };
        if let Err(err) = write_event(&mut self.writer, &event) {
            log::warn!("wayland_probe_write_failed err={err:#}");
            self.running = false;
        } else {
            self.event_count = self.event_count.saturating_add(1);
        }
    }

    pub(super) fn write_discarded(&mut self, commit_ns: u64) {
        let event = WaylandPresentationEventRecord {
            elapsed_ms: commit_ns
                .checked_sub(self.start_monotonic_ns)
                .map(|ns| ns / 1_000_000)
                .unwrap_or_else(|| self.started.elapsed().as_millis() as u64),
            source: "self_test".to_owned(),
            app_id: Some("stutter".to_owned()),
            surface_role: Some("self_test".to_owned()),
            commit_ns: Some(commit_ns),
            discarded: true,
            confidence: "low".to_owned(),
            ..Default::default()
        };
        if let Err(err) = write_event(&mut self.writer, &event) {
            log::warn!("wayland_probe_write_failed err={err:#}");
            self.running = false;
        } else {
            self.event_count = self.event_count.saturating_add(1);
        }
    }
}

pub(super) fn create_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<State>,
) -> Result<(wl_shm_pool::WlShmPool, wl_buffer::WlBuffer)> {
    let size = (WIDTH * HEIGHT * 4) as usize;
    let mut file = super::memfd::create_memfd(size)?;
    draw(&mut file, WIDTH, HEIGHT)?;
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        WIDTH as i32,
        HEIGHT as i32,
        (WIDTH * 4) as i32,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    Ok((pool, buffer))
}

fn draw(file: &mut File, width: u32, height: u32) -> Result<()> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255) / width) as u8;
            let g = ((y * 255) / height) as u8;
            let b = (((x + y) * 255) / (width + height)) as u8;
            buf.extend_from_slice(&[b, g, r, 0xff]);
        }
    }
    file.write_all(&buf)?;
    Ok(())
}

fn write_event(writer: &mut BufWriter<File>, event: &WaylandPresentationEventRecord) -> Result<()> {
    serde_json::to_writer(&mut *writer, event)?;
    writeln!(writer)?;
    Ok(())
}

fn presentation_flags(flags: WEnum<wp_presentation_feedback::Kind>) -> (Option<bool>, Vec<String>) {
    match flags {
        WEnum::Value(value) => {
            let mut names = Vec::new();
            if value.contains(wp_presentation_feedback::Kind::Vsync) {
                names.push("vsync".to_owned());
            }
            if value.contains(wp_presentation_feedback::Kind::HwClock) {
                names.push("hw_clock".to_owned());
            }
            if value.contains(wp_presentation_feedback::Kind::HwCompletion) {
                names.push("hw_completion".to_owned());
            }
            let zero_copy = value.contains(wp_presentation_feedback::Kind::ZeroCopy);
            if zero_copy {
                names.push("zero_copy".to_owned());
            }
            (Some(zero_copy), names)
        }
        WEnum::Unknown(raw) => (None, vec![format!("unknown:{raw}")]),
    }
}
