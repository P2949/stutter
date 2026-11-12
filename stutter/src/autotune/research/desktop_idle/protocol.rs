//! Bounded passive Wayland idle protocol consumer. False means idle is not established.
use std::{os::unix::net::UnixStream, time::Duration};

use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    backend::WaylandError,
    delegate_noop,
    protocol::{wl_callback, wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

use super::DesktopIdleObservation;

#[derive(Clone, Copy)]
enum SyncPoint {
    Globals,
    Ready,
}

#[derive(Clone, Copy)]
enum IdleKind {
    InhibitorAware,
    InputOnly,
}

#[derive(Default)]
struct State {
    seats: Vec<(u32, u32)>,
    notifiers: Vec<(u32, u32)>,
    bound: Option<(u32, u32)>,
    globals_ready: bool,
    setup_ready: bool,
    caps: Option<wl_seat::Capability>,
    inhibitor_idle: bool,
    input_idle: bool,
    invalid: bool,
    dirty: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                match interface.as_str() {
                    "wl_seat" => state.seats.push((name, version)),
                    "ext_idle_notifier_v1" => state.notifiers.push((name, version)),
                    _ => return,
                }
                if state.bound.is_some() {
                    state.invalid = true;
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.seats.retain(|(id, _)| *id != name);
                state.notifiers.retain(|(id, _)| *id != name);
                if state
                    .bound
                    .is_some_and(|(seat, notifier)| name == seat || name == notifier)
                {
                    state.invalid = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, SyncPoint> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        point: &SyncPoint,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            match point {
                SyncPoint::Globals => state.globals_ready = true,
                SyncPoint::Ready => state.setup_ready = true,
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            match capabilities {
                WEnum::Value(caps) => state.caps = Some(caps),
                WEnum::Unknown(_) => state.invalid = true,
            }
            if state.setup_ready && !state.seat_usable() {
                state.invalid = true;
            }
        }
    }
}

delegate_noop!(State: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, IdleKind> for State {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        kind: &IdleKind,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let idle = match kind {
            IdleKind::InhibitorAware => &mut state.inhibitor_idle,
            IdleKind::InputOnly => &mut state.input_idle,
        };
        match event {
            ext_idle_notification_v1::Event::Idled if !*idle => *idle = true,
            ext_idle_notification_v1::Event::Resumed if *idle => *idle = false,
            _ => state.invalid = true,
        }
        state.dirty = true;
    }
}

impl State {
    fn seat_usable(&self) -> bool {
        self.caps.is_some_and(|caps| {
            caps.contains(wl_seat::Capability::Keyboard | wl_seat::Capability::Pointer)
        })
    }
}

fn publish(updates: &mpsc::Sender<DesktopIdleObservation>, state: &State) -> anyhow::Result<()> {
    let value = if state.setup_ready && state.seat_usable() && !state.invalid {
        DesktopIdleObservation::Monitoring {
            inhibitor_idle: state.inhibitor_idle,
            input_idle: state.input_idle,
        }
    } else {
        DesktopIdleObservation::Unknown
    };
    updates
        .try_send(value)
        .map_err(|_| anyhow::anyhow!("idle observation consumer unavailable"))
}

pub(super) async fn observe(
    stream: UnixStream,
    idle_millis: u32,
    updates: mpsc::Sender<DesktopIdleObservation>,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    stream.set_nonblocking(true)?;
    let readiness = crate::syscall::register_unix_stream_readiness(stream.try_clone()?)?;
    let conn = Connection::from_socket(stream)?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let display = conn.display();
    let registry = display.get_registry(&qh, ());
    display.sync(&qh, SyncPoint::Globals);
    let mut state = State::default();
    let mut resources = None;
    let start = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    publish(&updates, &state)?;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        queue.dispatch_pending(&mut state)?;
        anyhow::ensure!(
            !state.invalid,
            "idle protocol identity or event sequence changed"
        );
        if state.globals_ready && state.bound.is_none() {
            anyhow::ensure!(
                state.seats.len() == 1 && state.notifiers.len() == 1 && state.notifiers[0].1 >= 2,
                "idle protocol or unique seat unavailable"
            );
            let (seat_id, seat_version) = state.seats[0];
            let (notifier_id, _) = state.notifiers[0];
            let seat =
                registry.bind::<wl_seat::WlSeat, _, _>(seat_id, seat_version.min(7), &qh, ());
            let notifier = registry.bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(
                notifier_id,
                2,
                &qh,
                (),
            );
            let inhibitor =
                notifier.get_idle_notification(idle_millis, &seat, &qh, IdleKind::InhibitorAware);
            let input =
                notifier.get_input_idle_notification(idle_millis, &seat, &qh, IdleKind::InputOnly);
            state.bound = Some((seat_id, notifier_id));
            resources = Some((seat, notifier, inhibitor, input));
            display.sync(&qh, SyncPoint::Ready);
        }
        if state.setup_ready {
            anyhow::ensure!(
                state.seat_usable() && resources.is_some(),
                "idle seat unavailable"
            );
        } else {
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(3),
                "idle protocol setup timed out"
            );
        }
        if state.dirty {
            publish(&updates, &state)?;
            state.dirty = false;
        }
        queue.flush()?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        tokio::select! {
            ready = readiness.readable() => {
                let mut ready = ready?;
                match guard.read() {
                    Ok(_) => {},
                    Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => ready.clear_ready(),
                    Err(error) => return Err(error.into()),
                }
            }
            _ = heartbeat.tick() => { drop(guard); publish(&updates, &state)?; }
            changed = stop.changed() => {
                drop(guard);
                if changed.is_err() || *stop.borrow() { return Ok(()); }
            }
        }
    }
}
