use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::{
    wp::presentation_time::client::{wp_presentation, wp_presentation_feedback},
    xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};

use super::snapshot::{State, create_buffer};

pub(super) struct FrameFeedbackData {
    pub(super) commit_ns: u64,
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
