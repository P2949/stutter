use std::{fs, path::PathBuf};

use super::*;

fn listing(name: &str, rows: &[(u64, u64, u64, u64)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-regions-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (index, (start, end, accesses, age)) in rows.iter().enumerate() {
        let dir = root.join(index.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("start"), start.to_string()).unwrap();
        fs::write(dir.join("end"), end.to_string()).unwrap();
        fs::write(dir.join("nr_accesses"), accesses.to_string()).unwrap();
        fs::write(dir.join("age"), age.to_string()).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn regions_are_read_and_summarized_into_hot_and_cold_bytes() {
    let root = listing(
        "mix",
        &[
            (0, 4096, 20, 1),
            (4096, 12288, 0, 50),
            (12288, 16384, 0, 90),
            (16384, 20480, 3, 4),
        ],
    );
    let (regions, truncated) = read_tried_regions(&root).unwrap();
    assert!(!truncated);
    let summary = summarize(&regions, truncated, 10);
    assert_eq!(summary.regions, 4);
    assert_eq!(summary.total_bytes, 20480);
    assert_eq!(summary.hot_bytes, 4096);
    assert_eq!(summary.cold_bytes, 12288);
    assert_eq!(summary.hottest[0].start, 0);
    // Among equally idle regions the longest-idle one is named first.
    assert_eq!(summary.coldest[0].age, 90);
    assert_eq!(summary.coldest[1].age, 50);
}

#[test]
fn malformed_and_inverted_regions_are_skipped_and_a_missing_listing_is_none() {
    let root = listing("bad", &[(100, 50, 1, 1), (0, 4096, 1, 1)]);
    fs::write(root.join("1/age"), "not a number").unwrap();
    let (regions, _) = read_tried_regions(&root).unwrap();
    assert!(regions.is_empty(), "{regions:?}");
    assert_eq!(summarize(&regions, false, 1).total_bytes, 0);
    assert!(read_tried_regions(&root.join("absent")).is_none());
}

#[test]
fn a_huge_listing_is_read_only_up_to_the_bound_and_says_so() {
    let rows: Vec<(u64, u64, u64, u64)> = (0..(MAX_TRIED_REGIONS as u64 + 7))
        .map(|i| (i * 4096, i * 4096 + 4096, 1, 1))
        .collect();
    let root = listing("big", &rows);
    let (regions, truncated) = read_tried_regions(&root).unwrap();
    assert!(truncated);
    assert_eq!(regions.len(), MAX_TRIED_REGIONS);
    let summary = summarize(&regions, truncated, 1);
    assert!(summary.truncated);
    assert_eq!(summary.hottest.len(), NAMED_REGIONS);
    assert_eq!(summary.hot_bytes, MAX_TRIED_REGIONS as u64 * 4096);
}
