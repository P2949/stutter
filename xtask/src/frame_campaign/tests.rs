use super::*;

const MANIFEST_TEXT: &str = include_str!("../../../docs/validation/frame-graduation.toml");

#[test]
fn the_shipped_manifest_parses_and_each_block_yields_all_three_pair_kinds() {
    let spec = parse_manifest(MANIFEST_TEXT).unwrap();
    assert_eq!(spec.objective, "GameFramePacing");
    for block in 0..spec.calibration_blocks {
        let (light, heavy) = block_levels(&spec, block);
        assert!(light < heavy, "the lighter level has fewer busy loops");
        let runs: Vec<Run> = block_order(block)
            .into_iter()
            .enumerate()
            .map(|(position, condition)| Run {
                block,
                position: position as u32,
                condition,
                elapsed_ns: 1_000,
                execution_id: format!("{block}-{position}"),
            })
            .collect();
        let kinds: Vec<_> = pairs(&runs).iter().map(|pair| pair.kind).collect();
        assert_eq!(kinds.len(), 3, "block {block}: {kinds:?}");
        for kind in [
            stats::PairKind::Neutral,
            stats::PairKind::Beneficial,
            stats::PairKind::Regression,
        ] {
            assert!(kinds.contains(&kind), "block {block} lacks {kind:?}");
        }
    }
}

#[test]
fn a_malformed_manifest_is_rejected() {
    let bad = MANIFEST_TEXT.replace("levels_hogs = [0, 1, 3]", "levels_hogs = [3, 1]");
    assert!(parse_manifest(&bad).is_err());
    let bad = MANIFEST_TEXT.replace(
        "population_grid = [300, 600, 1200, 2400]",
        "population_grid = []",
    );
    assert!(parse_manifest(&bad).is_err());
    let bad = MANIFEST_TEXT.replace(
        "min_beneficial_keep_rate = 0.8",
        "min_beneficial_keep_rate = 0.0",
    );
    assert!(parse_manifest(&bad).is_err());
}

#[test]
fn the_selection_picks_the_smallest_population_and_threshold_that_satisfy_the_rule() {
    // A synthetic calibration set: neutral noise 2 percent, beneficial -50, regression +100.
    let spec = parse_manifest(MANIFEST_TEXT).unwrap();
    let mut runs = Vec::new();
    for block in 0..spec.calibration_blocks {
        let (cheap, costly) = (1_000_u64, 2_000_u64);
        for (position, condition) in block_order(block).into_iter().enumerate() {
            let base = if condition == Condition::Clean {
                cheap
            } else {
                costly
            };
            let value = base + (position as u64 % 2) * 20;
            runs.push(FrameRun {
                block,
                position: position as u32,
                condition,
                hogs: 0,
                valid_frames: 5_000,
                p99_us: spec.population_grid.iter().map(|&n| (n, value)).collect(),
                execution_id: format!("{block}-{position}"),
            });
        }
    }
    let calibration = pairs(&stats_runs(&runs, 300).unwrap());
    let (threshold, _) =
        select_threshold(&calibration, &spec.threshold_grid_percent, &spec.rule()).unwrap();
    // 2 percent noise times the 2.0 margin is 4 percent, so the smallest grid value (25) is chosen.
    assert_eq!(threshold, 25.0);
}
