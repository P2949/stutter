use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    time::{Duration, Instant},
};

use super::*;

const HEADER: &str = "os,cpu\nlinux,x\nfps,frametime,elapsed\n";

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-tail-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn session(dir: &std::path::Path, config: TailConfig) -> (MangoHudTailSession, Instant) {
    let now = Instant::now();
    let confined = ConfinedDir::open(dir, crate::syscall::geteuid()).unwrap();
    (
        MangoHudTailSession::new(confined, "log.csv", config, now),
        now,
    )
}

fn row(ms: u64) -> String {
    format!("60,16.6,{}\n", ms * 1_000_000)
}

fn drain(session: &mut MangoHudTailSession, now: Instant) -> Vec<TailedFrame> {
    let mut out = Vec::new();
    session.poll(now, usize::MAX, &mut out);
    out
}

#[test]
fn a_file_that_never_appears_fails_at_the_deadline() {
    let dir = dir("deadline");
    let (mut session, now) = session(&dir, TailConfig::default());
    assert_eq!(
        session.poll(now, 10, &mut Vec::new()),
        TailState::WaitingForFile
    );
    let late = now + Duration::from_secs(31);
    assert_eq!(
        session.poll(late, 10, &mut Vec::new()),
        TailState::Failed(TailFailure::NeverCreated)
    );
}

#[test]
fn header_metadata_partial_lines_and_crlf_are_handled_incrementally() {
    let dir = dir("incremental");
    fs::write(dir.join("log.csv"), HEADER).unwrap();
    let (mut session, now) = session(&dir, TailConfig::default());
    assert!(drain(&mut session, now).is_empty());
    assert_eq!(session.state(), TailState::Streaming);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(dir.join("log.csv"))
        .unwrap();
    write!(file, "60,16.6,1000").unwrap(); // partial: no newline yet
    assert!(drain(&mut session, now).is_empty());
    write!(file, "000\r\n{}", row(2)).unwrap();
    let frames = drain(&mut session, now);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].event.elapsed_ms, 1);
    assert_eq!(session.counters().frames, 2);
}

#[test]
fn a_full_downstream_leaves_frames_in_the_file_instead_of_dropping_them() {
    let dir = dir("backpressure");
    let mut text = HEADER.to_owned();
    for ms in 1..=50 {
        text.push_str(&row(ms * 20));
    }
    fs::write(dir.join("log.csv"), text).unwrap();
    let (mut session, now) = session(&dir, TailConfig::default());
    let mut all = Vec::new();
    for _ in 0..30 {
        let mut out = Vec::new();
        session.poll(now, 7, &mut out);
        assert!(out.len() <= 7);
        all.extend(out);
    }
    assert_eq!(all.len(), 50);
    assert!(
        all.windows(2)
            .all(|pair| pair[0].event.elapsed_ms < pair[1].event.elapsed_ms)
    );
}

#[test]
fn an_oversize_line_is_discarded_to_its_newline_and_counted() {
    let dir = dir("oversize");
    let config = TailConfig {
        max_line_bytes: 64,
        ..TailConfig::default()
    };
    fs::write(
        dir.join("log.csv"),
        format!("{HEADER}{}\n{}", "x".repeat(500), row(40)),
    )
    .unwrap();
    let (mut session, now) = session(&dir, config);
    let frames = drain(&mut session, now);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event.elapsed_ms, 40);
    assert_eq!(session.counters().oversize_lines_discarded, 1);
}

#[test]
fn truncation_starts_a_new_epoch_with_a_fresh_header() {
    let dir = dir("truncate");
    fs::write(
        dir.join("log.csv"),
        format!("{HEADER}{}{}", row(20), row(40)),
    )
    .unwrap();
    let (mut session, now) = session(&dir, TailConfig::default());
    assert_eq!(drain(&mut session, now).len(), 2);
    // Truncate and write a different layout: the schema is detected again.
    fs::write(dir.join("log.csv"), "frametime,elapsed_ms\n16.6,5\n").unwrap();
    let frames = drain(&mut session, now);
    assert_eq!(session.counters().truncations, 1);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].epoch, 1);
    assert_eq!(frames[0].event.elapsed_ms, 5);
}

#[test]
fn a_replaced_file_starts_a_new_epoch_and_changes_identity() {
    let dir = dir("replace");
    fs::write(dir.join("log.csv"), format!("{HEADER}{}", row(20))).unwrap();
    let (mut session, now) = session(&dir, TailConfig::default());
    assert_eq!(drain(&mut session, now).len(), 1);
    let first = session.identity().unwrap();
    fs::rename(dir.join("log.csv"), dir.join("old.csv")).unwrap();
    fs::write(dir.join("log.csv"), format!("{HEADER}{}{}", row(1), row(2))).unwrap();
    let frames = drain(&mut session, now);
    assert_eq!(session.counters().replacements, 1);
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame.epoch == 1));
    assert_ne!(session.identity().unwrap().ino, first.ino);
}

#[test]
fn a_symlinked_log_is_refused_and_a_headerless_log_fails() {
    let dir = dir("unsafe");
    symlink("/etc/passwd", dir.join("log.csv")).unwrap();
    let (mut linked, now) = session(&dir, TailConfig::default());
    assert_eq!(
        linked.poll(now, 5, &mut Vec::new()),
        TailState::Failed(TailFailure::UnsafeSource)
    );
    fs::remove_file(dir.join("log.csv")).unwrap();
    fs::write(dir.join("log.csv"), "a,b\n".repeat(100)).unwrap();
    let config = TailConfig {
        max_header_lines: 8,
        ..TailConfig::default()
    };
    let (mut headerless, now) = session(&dir, config);
    assert_eq!(
        headerless.poll(now, 5, &mut Vec::new()),
        TailState::Failed(TailFailure::NoHeader)
    );
}
