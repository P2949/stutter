use super::*;
use crate::autotune::research::{
    route_driver::RouteInput,
    scripted_input::{Key, KeyStep, PhaseScript, ScriptedInput},
};

fn live() -> LiveSession {
    LiveSession {
        wayland_display: "wayland-1".into(),
        x_display: ":0".into(),
        session_id: "11".into(),
    }
}

fn game(extra: &[(&str, &str)]) -> Vec<u8> {
    let mut env = vec![
        ("WAYLAND_DISPLAY", "wayland-77"),
        ("DISPLAY", ":77"),
        ("XDG_SESSION_ID", "42"),
    ];
    env.extend_from_slice(extra);
    env.into_iter()
        .flat_map(|(key, value)| format!("{key}={value}\0").into_bytes())
        .collect()
}

#[test]
fn only_a_game_on_a_display_and_session_of_its_own_is_an_isolated_seat() {
    let seat = attest(&game(&[]), &live()).unwrap();
    assert_eq!(seat.display(), "wayland-77");
    let refused = |environ: Vec<u8>, why: &'static str| {
        assert_eq!(attest(&environ, &live()).err(), Some(why));
    };
    // The live Wayland display, the live X display, the live session, or the live compositor itself.
    refused(
        game(&[("WAYLAND_DISPLAY", "wayland-1")]),
        "GameSharesTheLiveDisplay",
    );
    refused(game(&[("DISPLAY", ":0")]), "GameSharesTheLiveDisplay");
    refused(
        game(&[("XDG_SESSION_ID", "11")]),
        "GameSharesTheLiveSession",
    );
    refused(
        game(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")]),
        "GameOnTheLiveCompositor",
    );
    // A game whose displays or session are not shown is never assumed isolated.
    refused(b"HOME=/h\0".to_vec(), "GameDisplayUnknown");
    refused(
        b"WAYLAND_DISPLAY=wayland-77\0".to_vec(),
        "GameDisplayUnknown",
    );
    refused(
        b"WAYLAND_DISPLAY=wayland-77\0DISPLAY=:77\0".to_vec(),
        "GameSharesTheLiveSession",
    );
    // Nothing can be shown different from a live session whose own identifiers are unknown.
    assert_eq!(
        attest(
            &game(&[]),
            &LiveSession {
                session_id: String::new(),
                ..live()
            }
        )
        .err(),
        Some("LiveSessionUnknown")
    );
}

struct Device {
    display: String,
    written: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl EventSink for Device {
    fn write(&mut self, events: &[u8]) -> Result<(), &'static str> {
        self.written.lock().unwrap().extend_from_slice(events);
        Ok(())
    }
}

impl BoundSink for Device {
    fn bound_display(&self) -> &str {
        &self.display
    }
}

#[test]
fn a_sink_bound_to_any_other_display_is_refused_and_one_on_the_seat_carries_a_whole_route_and_releases_every_key()
 {
    let seat = attest(&game(&[]), &live()).unwrap();
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    // Bound to the live display: refused before a single event can be written.
    let on_live = Device {
        display: "wayland-1".into(),
        written: written.clone(),
    };
    assert_eq!(
        IsolatedSink::new(on_live, &seat).err(),
        Some("SinkNotOnTheIsolatedSeat")
    );
    assert!(written.lock().unwrap().is_empty());
    // Bound to the seat: the scripted route plays through it, and stopping it releases every key it pressed.
    let device = Device {
        display: "wayland-77".into(),
        written: written.clone(),
    };
    let sink = IsolatedSink::new(device, &seat).unwrap();
    let script = vec![PhaseScript {
        phase: "travel".into(),
        steps: vec![
            KeyStep {
                at_ms: 0,
                key: Key::W,
                down: true,
            },
            KeyStep {
                at_ms: 500,
                key: Key::W,
                down: false,
            },
        ],
    }];
    let mut input = ScriptedInput::new(sink, script);
    assert!(input.available());
    input.begin_phase("travel").unwrap();
    input.tick(600).unwrap();
    input.stop();
    assert!(input.released(), "no key is left held");
    let bytes = written.lock().unwrap().clone();
    assert!(
        !bytes.is_empty()
            && bytes.len() % crate::autotune::research::scripted_input::EVENT_BYTES == 0
    );
}
