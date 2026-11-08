use std::{fs, path::PathBuf};

use super::*;

fn process(root: &Path, pid: u32, ppid: u32, start: u64, maps: &str) {
    let dir = root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    let zeros = vec!["0"; 17].join(" ");
    fs::write(
        dir.join("stat"),
        format!("{pid} (p) S {ppid} {zeros} {start} 0 0\n"),
    )
    .unwrap();
    fs::write(dir.join("maps"), maps).unwrap();
}

fn tree(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-render-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

const HUD: &str = "7f00 r-xp /usr/lib/libMangoHud.so\n";
const PLAIN: &str = "7f00 r-xp /usr/lib/libc.so.6\n";

#[test]
fn exactly_the_mangohud_renderers_inside_the_launch_tree_are_counted() {
    let root = tree("count");
    let uid = crate::syscall::geteuid();
    process(&root, 100, 1, 5, PLAIN); // the launcher
    process(&root, 101, 100, 6, HUD); // a renderer under it
    process(&root, 102, 101, 7, PLAIN); // a helper under the renderer
    process(&root, 103, 1, 8, HUD); // an unrelated MangoHud process
    assert_eq!(count_mangohud_renderers(&root, 100, 5, uid), 1);
    process(&root, 104, 100, 9, HUD); // a second renderer: now ambiguous
    assert_eq!(count_mangohud_renderers(&root, 100, 5, uid), 2);
    // A reused launcher pid (different start time) matches nothing.
    assert_eq!(count_mangohud_renderers(&root, 100, 999, uid), 0);
    // Another user's tree is not ours to count.
    assert_eq!(
        count_mangohud_renderers(&root, 100, 5, uid.wrapping_add(1)),
        0
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_launcher_that_is_itself_the_renderer_counts_and_a_missing_proc_counts_none() {
    let root = tree("self");
    let uid = crate::syscall::geteuid();
    process(&root, 200, 1, 5, HUD);
    assert_eq!(count_mangohud_renderers(&root, 200, 5, uid), 1);
    assert_eq!(
        count_mangohud_renderers(&root.join("absent"), 200, 5, uid),
        0
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_recent_non_summary_csv_logs_count_as_live() {
    let dir = tree("logs");
    fs::write(dir.join("game_a.csv"), "x").unwrap();
    fs::write(dir.join("game_b.csv"), "x").unwrap();
    fs::write(dir.join("game_a_summary.csv"), "x").unwrap();
    fs::write(dir.join("notes.txt"), "x").unwrap();
    assert_eq!(live_log_count(&dir, Duration::from_secs(60)), 2);
    // A zero window makes every log "old".
    assert_eq!(live_log_count(&dir, Duration::from_secs(0)), 0);
    assert_eq!(
        live_log_count(&dir.join("absent"), Duration::from_secs(60)),
        0
    );
    fs::remove_dir_all(&dir).unwrap();
}
