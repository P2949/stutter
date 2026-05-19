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
    use std::{
        fs::{self, File},
        io::{BufWriter, Write},
        os::fd::{AsFd, FromRawFd},
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use wayland_client::{
        Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
        },
    };
    use wayland_protocols::{
        wp::presentation_time::client::{wp_presentation, wp_presentation_feedback},
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    };

    use super::WaylandProbeCommandInput;
    use crate::recorder::WaylandPresentationEventRecord;

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 240;

    pub(super) fn run(input: WaylandProbeCommandInput) -> Result<()> {
        fs::create_dir_all(&input.out_dir)
            .with_context(|| format!("failed to create {}", input.out_dir.display()))?;
        let events_path = input.out_dir.join("wayland_presentation_events.json");
        let writer = BufWriter::new(
            File::create(&events_path)
                .with_context(|| format!("failed to create {}", events_path.display()))?,
        );

        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let start_monotonic_ns = monotonic_now_ns();
        let mut state = State {
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

    struct State {
        running: bool,
        duration: Duration,
        started: Instant,
        start_monotonic_ns: u64,
        output_filter: Option<String>,
        fullscreen: bool,
        base_surface: Option<wl_surface::WlSurface>,
        buffer: Option<wl_buffer::WlBuffer>,
        shm_pool: Option<wl_shm_pool::WlShmPool>,
        wm_base: Option<xdg_wm_base::XdgWmBase>,
        xdg_surface: Option<xdg_surface::XdgSurface>,
        toplevel: Option<xdg_toplevel::XdgToplevel>,
        presentation: Option<wp_presentation::WpPresentation>,
        configured: bool,
        pending_feedback: bool,
        frame_index: u64,
        selected_output: Option<wl_output::WlOutput>,
        writer: BufWriter<File>,
        event_count: u64,
    }

    impl State {
        fn validate_globals(&self) -> Result<()> {
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

        fn maybe_init_xdg_surface(&mut self, qh: &QueueHandle<State>) {
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

        fn maybe_submit_frame(&mut self, qh: &QueueHandle<State>) {
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

            let commit_ns = monotonic_now_ns();
            presentation.feedback(surface, qh, FrameFeedbackData { commit_ns });
            surface.attach(Some(buffer), 0, 0);
            surface.damage(0, 0, WIDTH as i32, HEIGHT as i32);
            surface.commit();
            self.pending_feedback = true;
            self.frame_index = self.frame_index.saturating_add(1);
        }

        fn write_presented(
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

        fn write_discarded(&mut self, commit_ns: u64) {
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

    struct FrameFeedbackData {
        commit_ns: u64,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_compositor" => {
                        let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                            name,
                            version.min(4),
                            qh,
                            (),
                        );
                        state.base_surface = Some(compositor.create_surface(qh, ()));
                        state.maybe_init_xdg_surface(qh);
                    }
                    "wl_shm" => {
                        let shm = registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ());
                        match create_buffer(&shm, qh) {
                            Ok((pool, buffer)) => {
                                state.shm_pool = Some(pool);
                                state.buffer = Some(buffer);
                                state.maybe_submit_frame(qh);
                            }
                            Err(err) => {
                                log::warn!("wayland_probe_buffer_failed err={err:#}");
                                state.running = false;
                            }
                        }
                    }
                    "wl_output" => {
                        let bind_version = version.min(4);
                        registry.bind::<wl_output::WlOutput, _, _>(name, bind_version, qh, ());
                    }
                    "xdg_wm_base" => {
                        state.wm_base =
                            Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                        state.maybe_init_xdg_surface(qh);
                    }
                    "wp_presentation" => {
                        state.presentation = Some(
                            registry.bind::<wp_presentation::WpPresentation, _, _>(name, 1, qh, ()),
                        );
                        state.maybe_submit_frame(qh);
                    }
                    _ => {}
                }
            }
        }
    }

    delegate_noop!(State: ignore wl_compositor::WlCompositor);
    delegate_noop!(State: ignore wl_surface::WlSurface);
    delegate_noop!(State: ignore wl_shm::WlShm);
    delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
    delegate_noop!(State: ignore wl_buffer::WlBuffer);

    impl Dispatch<wl_output::WlOutput, ()> for State {
        fn event(
            state: &mut Self,
            output: &wl_output::WlOutput,
            event: wl_output::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_output::Event::Name { name } = event
                && state
                    .output_filter
                    .as_ref()
                    .is_some_and(|requested| requested == &name)
            {
                state.selected_output = Some(output.clone());
                if state.fullscreen
                    && let Some(toplevel) = state.toplevel.as_ref()
                {
                    toplevel.set_fullscreen(state.selected_output.as_ref());
                }
            }
        }
    }

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
        fn event(
            _: &mut Self,
            wm_base: &xdg_wm_base::XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                wm_base.pong(serial);
            }
        }
    }

    impl Dispatch<xdg_surface::XdgSurface, ()> for State {
        fn event(
            state: &mut Self,
            xdg_surface: &xdg_surface::XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial, .. } = event {
                xdg_surface.ack_configure(serial);
                state.configured = true;
                state.maybe_submit_frame(qh);
            }
        }
    }

    impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
        fn event(
            state: &mut Self,
            _: &xdg_toplevel::XdgToplevel,
            event: xdg_toplevel::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_toplevel::Event::Close = event {
                state.running = false;
            }
        }
    }

    impl Dispatch<wp_presentation::WpPresentation, ()> for State {
        fn event(
            _: &mut Self,
            _: &wp_presentation::WpPresentation,
            _: wp_presentation::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, FrameFeedbackData> for State {
        fn event(
            state: &mut Self,
            _: &wp_presentation_feedback::WpPresentationFeedback,
            event: wp_presentation_feedback::Event,
            data: &FrameFeedbackData,
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            match event {
                wp_presentation_feedback::Event::Presented {
                    tv_sec_hi,
                    tv_sec_lo,
                    tv_nsec,
                    refresh,
                    seq_hi,
                    seq_lo,
                    flags,
                } => {
                    let sec = ((tv_sec_hi as u64) << 32) | tv_sec_lo as u64;
                    let presented_ns = sec
                        .saturating_mul(1_000_000_000)
                        .saturating_add(tv_nsec as u64);
                    let sequence = ((seq_hi as u64) << 32) | seq_lo as u64;
                    state.write_presented(data.commit_ns, presented_ns, refresh, sequence, flags);
                    state.pending_feedback = false;
                    state.maybe_submit_frame(qh);
                }
                wp_presentation_feedback::Event::Discarded => {
                    state.write_discarded(data.commit_ns);
                    state.pending_feedback = false;
                    state.maybe_submit_frame(qh);
                }
                _ => {}
            }
        }
    }

    fn create_buffer(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<State>,
    ) -> Result<(wl_shm_pool::WlShmPool, wl_buffer::WlBuffer)> {
        let size = (WIDTH * HEIGHT * 4) as usize;
        let mut file = create_memfd(size)?;
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

    fn write_event(
        writer: &mut BufWriter<File>,
        event: &WaylandPresentationEventRecord,
    ) -> Result<()> {
        serde_json::to_writer(&mut *writer, event)?;
        writeln!(writer)?;
        Ok(())
    }

    fn create_memfd(size: usize) -> Result<File> {
        let name = c"stutter-wayland-probe";
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("memfd_create failed");
        }
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(size as u64)?;
        Ok(file)
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

    fn monotonic_now_ns() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts.tv_nsec as u64)
    }

    fn presentation_flags(
        flags: WEnum<wp_presentation_feedback::Kind>,
    ) -> (Option<bool>, Vec<String>) {
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
}
