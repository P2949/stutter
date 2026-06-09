use super::*;

#[test]
fn tree_tick_not_needed_for_direct_pid_only() {
    assert!(!needs_tree_tick_from_parts(false, false, false));
}

#[test]
fn tree_tick_interval_uses_watch_poll_ms() {
    let mut config = MonitorConfig::default();
    config.watch.poll_ms = 1_337;

    assert_eq!(
        crate::session::targeting::tree_tick_interval_ms(&config),
        1_337
    );
}

#[test]
fn tree_tick_needed_for_tree_roots() {
    assert!(needs_tree_tick_from_parts(true, false, false));
}

#[test]
fn tree_tick_needed_for_watch_process_even_without_current_root() {
    assert!(needs_tree_tick_from_parts(false, true, false));
}

#[test]
fn tree_tick_needed_for_cgroupv2() {
    assert!(needs_tree_tick_from_parts(false, false, true));
}

#[test]
fn mangohud_alignment_receiver_stays_pending_after_successful_alignment() {
    use std::{
        future::Future,
        task::{Context, Poll},
    };

    use futures_util::task::noop_waker_ref;

    let (tx, rx) = tokio::sync::oneshot::channel::<MangoHudAlignment>();
    let mut rx = Box::pin(fused_mangohud_alignment_receiver(rx));

    tx.send((0, 2_063_518_675_341))
        .expect("test receiver should still be alive");

    let mut cx = Context::from_waker(noop_waker_ref());

    match rx.as_mut().poll(&mut cx) {
        Poll::Ready(Ok((raw_ms, monotonic_ns))) => {
            assert_eq!(raw_ms, 0);
            assert_eq!(monotonic_ns, 2_063_518_675_341);
        }
        other => panic!("expected first poll to yield MangoHud alignment, got {other:?}"),
    }

    match rx.as_mut().poll(&mut cx) {
        Poll::Pending => {}
        other => panic!("completed MangoHud alignment receiver must stay pending, got {other:?}"),
    }
}

#[test]
fn mangohud_alignment_receiver_stays_pending_after_alignment_task_drops_sender() {
    use std::{
        future::Future,
        task::{Context, Poll},
    };

    use futures_util::task::noop_waker_ref;

    let (tx, rx) = tokio::sync::oneshot::channel::<MangoHudAlignment>();
    let mut rx = Box::pin(fused_mangohud_alignment_receiver(rx));

    drop(tx);

    let mut cx = Context::from_waker(noop_waker_ref());

    match rx.as_mut().poll(&mut cx) {
        Poll::Ready(Err(_)) => {}
        other => {
            panic!("expected first poll to observe closed MangoHud alignment sender, got {other:?}")
        }
    }

    match rx.as_mut().poll(&mut cx) {
        Poll::Pending => {}
        other => panic!("closed MangoHud alignment receiver must stay pending, got {other:?}"),
    }
}
