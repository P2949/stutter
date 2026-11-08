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
    assert_eq!(mangohud_renderers(&root, 100, 5, uid).found.len(), 1);
    process(&root, 104, 100, 9, HUD); // a second renderer: now ambiguous
    assert_eq!(mangohud_renderers(&root, 100, 5, uid).found.len(), 2);
    // A reused launcher pid (different start time) matches nothing.
    assert_eq!(mangohud_renderers(&root, 100, 999, uid).found.len(), 0);
    // Another user's tree is not ours to count.
    assert!(
        mangohud_renderers(&root, 100, 5, uid.wrapping_add(1))
            .found
            .is_empty()
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_launcher_that_is_itself_the_renderer_counts_and_a_missing_proc_counts_none() {
    let root = tree("self");
    let uid = crate::syscall::geteuid();
    process(&root, 200, 1, 5, HUD);
    assert_eq!(mangohud_renderers(&root, 200, 5, uid).found.len(), 1);
    assert!(mangohud_renderers(&root.join("absent"), 200, 5, uid) == RendererScan::default());
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

#[test]
fn an_unreadable_or_truncated_maps_or_a_scan_limit_means_uniqueness_is_not_proven() {
    let uid = crate::syscall::geteuid();
    // One visible renderer, and a second in-tree process whose `maps` cannot be read at all.
    let root = tree("unreadable");
    process(&root, 100, 1, 5, PLAIN);
    process(&root, 101, 100, 6, HUD);
    process(&root, 102, 100, 7, PLAIN);
    fs::remove_file(root.join("102/maps")).unwrap();
    let scan = mangohud_renderers(&root, 100, 5, uid);
    assert_eq!(scan.found, vec![(101, 6)]);
    assert!(
        !scan.complete,
        "an unreadable maps cannot rule a renderer out"
    );
    assert_eq!(
        scan.sole(),
        None,
        "one found in an incomplete scan is not a unique producer"
    );
    fs::remove_dir_all(&root).unwrap();

    // A `maps` longer than the bound that never mentions MangoHud inside it: the rest is unseen.
    let root = tree("truncated");
    process(&root, 100, 1, 5, PLAIN);
    process(&root, 101, 100, 6, HUD);
    process(&root, 102, 100, 7, PLAIN);
    fs::write(
        root.join("102/maps"),
        vec![b'x'; (MAX_MAPS_BYTES + 16) as usize],
    )
    .unwrap();
    let scan = mangohud_renderers(&root, 100, 5, uid);
    assert!(!scan.complete);
    assert_eq!(scan.sole(), None);
    // But a mention inside the bound is a mention, whatever follows.
    let mut mention = HUD.as_bytes().to_vec();
    mention.extend(vec![b'x'; (MAX_MAPS_BYTES + 16) as usize]);
    fs::write(root.join("102/maps"), mention).unwrap();
    assert_eq!(mangohud_renderers(&root, 100, 5, uid).found.len(), 2);
    fs::remove_dir_all(&root).unwrap();

    // The entry limit: a scan that stopped early cannot say it saw every process.
    let root = tree("limit");
    for pid in 100..110u32 {
        process(
            &root,
            pid,
            if pid == 100 { 1 } else { 100 },
            u64::from(pid),
            PLAIN,
        );
    }
    fs::write(root.join("105/maps"), HUD).unwrap();
    let limited = scan_with_limit(&root, 100, 100, uid, 3);
    assert!(!limited.complete);
    assert_eq!(limited.sole(), None);
    let full = scan_with_limit(&root, 100, 100, uid, 1000);
    assert!(full.complete);
    assert_eq!(full.sole(), Some((105, 105)));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_fully_inspected_single_renderer_is_the_sole_producer() {
    let uid = crate::syscall::geteuid();
    let root = tree("sole");
    process(&root, 100, 1, 5, PLAIN);
    process(&root, 101, 100, 6, HUD);
    let scan = mangohud_renderers(&root, 100, 5, uid);
    assert!(scan.complete);
    assert_eq!(scan.sole(), Some((101, 6)));
    fs::remove_dir_all(&root).unwrap();
}
