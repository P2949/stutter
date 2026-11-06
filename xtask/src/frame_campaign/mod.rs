//! Calibrating and graduating the frame-pacing decision rule from measured pairs.
//!
//! The parameters are pre-registered in `docs/validation/frame-graduation.toml`. The statistics and
//! the freeze-before-holdout discipline are those of the compile-throughput objective campaign; only
//! the measurement differs: a pinned CPU-bound `vkcube` logged by MangoHud, whose frame p99 is read
//! with the controller's own parser and rolling window.
mod evidence;
mod measure;
#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::objective_campaign::{
    read_json, stats,
    stats::{Condition, Pair, Rule, Run, holdout_passes, pairs, select_threshold, tally},
    write_json,
};

pub const MANIFEST: &str = "docs/validation/frame-graduation.toml";

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct CampaignArgs {
    /// Durable output directory; an existing one is resumed.
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u32,
    frame_pacing: Spec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    pub objective: String,
    pub cpu: u32,
    pub seconds: u64,
    pub levels_hogs: Vec<u32>,
    pub calibration_blocks: u32,
    pub holdout_blocks: u32,
    pub population_grid: Vec<usize>,
    pub threshold_grid_percent: Vec<f64>,
    pub noise_margin: f64,
    pub min_beneficial_keep_rate: f64,
    pub max_false_keeps: u32,
}

impl Spec {
    fn rule(&self) -> Rule {
        Rule {
            noise_margin: self.noise_margin,
            min_beneficial_keep_rate: self.min_beneficial_keep_rate,
            max_false_keeps: self.max_false_keeps,
        }
    }
}

/// One measured run. `p99_us` maps each pre-registered population to the frame p99 (microseconds)
/// over that many frames, so the population can be chosen on calibration data without re-measuring.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct FrameRun {
    pub block: u32,
    pub position: u32,
    pub condition: Condition,
    pub hogs: u32,
    pub valid_frames: usize,
    pub p99_us: std::collections::BTreeMap<usize, u64>,
    pub execution_id: String,
}

pub(crate) fn parse_manifest(text: &str) -> anyhow::Result<Spec> {
    let file: File = toml::from_str(text).context("parse the frame campaign manifest")?;
    ensure!(file.schema_version == 1, "unsupported manifest schema");
    let spec = file.frame_pacing;
    ensure!(
        spec.levels_hogs.len() >= 2 && spec.levels_hogs.windows(2).all(|w| w[0] < w[1]),
        "levels_hogs must be strictly increasing (lightest interference first)"
    );
    ensure!(
        spec.calibration_blocks > 0 && spec.holdout_blocks > 0 && spec.seconds >= 5,
        "blocks and run length must be positive"
    );
    ensure!(
        !spec.population_grid.is_empty()
            && spec.population_grid.windows(2).all(|w| w[0] < w[1])
            && spec.population_grid[0] > 0,
        "population_grid must be positive and ascending"
    );
    ensure!(
        !spec.threshold_grid_percent.is_empty()
            && spec.threshold_grid_percent.windows(2).all(|w| w[0] < w[1])
            && spec.threshold_grid_percent[0] > 0.0,
        "threshold_grid_percent must be positive and ascending"
    );
    ensure!(
        spec.noise_margin >= 1.0
            && (0.0..=1.0).contains(&spec.min_beneficial_keep_rate)
            && spec.min_beneficial_keep_rate > 0.0,
        "selection rule out of range"
    );
    Ok(spec)
}

/// The lighter and heavier interference levels (busy-loop counts) compared by a block.
pub(crate) fn block_levels(spec: &Spec, block: u32) -> (u32, u32) {
    let pair = (block as usize) % (spec.levels_hogs.len() - 1);
    (spec.levels_hogs[pair], spec.levels_hogs[pair + 1])
}

/// The conditions of a block in run order; the order alternates so neither direction of a
/// comparison is always measured first.
pub(crate) fn block_order(block: u32) -> [Condition; 4] {
    use Condition::{Clean as C, Interfered as I};
    if block.is_multiple_of(2) {
        [C, C, I, C]
    } else {
        [C, I, C, C]
    }
}

fn run_path(out: &Path, block: u32, position: u32) -> PathBuf {
    out.join("runs").join(format!("{block:03}-{position}.json"))
}

pub(crate) fn load_runs(out: &Path, blocks: std::ops::Range<u32>) -> anyhow::Result<Vec<FrameRun>> {
    let mut runs = Vec::new();
    for block in blocks {
        for position in 0..4 {
            let path = run_path(out, block, position);
            runs.push(
                serde_json::from_value(read_json(&path)?)
                    .with_context(|| format!("run {}", path.display()))?,
            );
        }
    }
    Ok(runs)
}

/// The runs as the shared statistics see them: the p99 at one population stands in for the metric.
pub(crate) fn stats_runs(runs: &[FrameRun], population: usize) -> anyhow::Result<Vec<Run>> {
    runs.iter()
        .map(|run| {
            Ok(Run {
                block: run.block,
                position: run.position,
                condition: run.condition,
                elapsed_ns: *run
                    .p99_us
                    .get(&population)
                    .with_context(|| format!("no p99 at population {population}"))?,
                execution_id: run.execution_id.clone(),
            })
        })
        .collect()
}

fn collect(out: &Path, spec: &Spec, blocks: std::ops::Range<u32>) -> anyhow::Result<()> {
    for block in blocks {
        for (position, condition) in block_order(block).into_iter().enumerate() {
            let position = position as u32;
            if !run_path(out, block, position).exists() {
                measure::execute_run(out, spec, block, position, condition)?;
            }
        }
    }
    Ok(())
}

/// Chooses the smallest population and then the smallest threshold that satisfy the rule on
/// calibration pairs, and writes both down. Refuses to run once any holdout run exists without a
/// frozen choice, so the holdout can never inform the selection.
fn freeze(out: &Path, spec: &Spec) -> anyhow::Result<Value> {
    let frozen = out.join("frozen.json");
    if frozen.exists() {
        return read_json(&frozen);
    }
    ensure!(
        !run_path(out, spec.calibration_blocks, 0).exists(),
        "holdout data exists but no threshold was frozen; the holdout is contaminated"
    );
    let runs = load_runs(out, 0..spec.calibration_blocks)?;
    let mut considered = Vec::new();
    for &population in &spec.population_grid {
        let calibration = pairs(&stats_runs(&runs, population)?);
        match select_threshold(&calibration, &spec.threshold_grid_percent, &spec.rule()) {
            Some((threshold, tally)) => {
                let value = json!({
                    "population": population,
                    "threshold_percent": threshold,
                    "calibration_tally": tally,
                    "calibration_keep_rate": tally.keep_rate(),
                    "neutral_noise_percent": stats::neutral_noise_percent(&calibration),
                    "calibration_pairs": calibration,
                    "populations_considered": considered,
                    "rule": {"noise_margin": spec.noise_margin, "min_beneficial_keep_rate": spec.min_beneficial_keep_rate, "max_false_keeps": spec.max_false_keeps},
                });
                write_json(&frozen, &value)?;
                println!("frozen: population {population}, threshold {threshold} percent");
                return Ok(value);
            }
            None => considered.push(json!({
                "population": population,
                "neutral_noise_percent": stats::neutral_noise_percent(&calibration),
                "satisfied": false,
            })),
        }
    }
    write_json(
        &out.join("unproven.json"),
        &json!({"reason": "no pre-registered population and threshold satisfy the selection rule on calibration", "populations_considered": considered}),
    )?;
    bail!(
        "no pre-registered population and threshold satisfy the rule: the objective stays unproven (DeferredEvidence)"
    )
}

/// The single holdout evaluation. An existing result is returned as it is and never recomputed.
fn evaluate(out: &Path, spec: &Spec, population: usize, threshold: f64) -> anyhow::Result<Value> {
    let path = out.join("holdout.json");
    if path.exists() {
        return read_json(&path);
    }
    let range = spec.calibration_blocks..spec.calibration_blocks + spec.holdout_blocks;
    let holdout: Vec<Pair> = pairs(&stats_runs(&load_runs(out, range)?, population)?);
    let result = tally(&holdout, threshold);
    let passed = holdout_passes(&result, &spec.rule());
    let value = json!({
        "population": population,
        "threshold_percent": threshold,
        "holdout_tally": result,
        "holdout_keep_rate": result.keep_rate(),
        "holdout_pairs": holdout,
        "passed": passed,
    });
    write_json(&path, &value)?;
    Ok(value)
}

pub fn run(root: &Path, args: &CampaignArgs) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()?;
    ensure!(
        status.stdout.is_empty(),
        "a frame campaign needs a clean committed source tree"
    );
    let spec = parse_manifest(&fs::read_to_string(root.join(MANIFEST))?)?;
    let commit = {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()?;
        ensure!(output.status.success(), "git rev-parse HEAD failed");
        String::from_utf8(output.stdout)?.trim().to_owned()
    };
    let manifest_sha256 =
        crate::validation_campaign::repository::file_sha256(&root.join(MANIFEST))?;
    fs::create_dir_all(args.out.join("runs"))?;
    let binding = args.out.join("campaign.json");
    if binding.exists() {
        let existing = read_json(&binding)?;
        ensure!(
            existing["commit"] == commit.as_str()
                && existing["manifest_sha256"] == manifest_sha256.as_str(),
            "this campaign directory belongs to another source or manifest"
        );
    } else {
        write_json(
            &binding,
            &json!({"commit": commit, "manifest_sha256": manifest_sha256, "objective": spec.objective}),
        )?;
    }
    collect(&args.out, &spec, 0..spec.calibration_blocks)?;
    let frozen = freeze(&args.out, &spec)?;
    let threshold = frozen["threshold_percent"]
        .as_f64()
        .context("no frozen threshold")?;
    let population = frozen["population"]
        .as_u64()
        .context("no frozen population")? as usize;
    collect(
        &args.out,
        &spec,
        spec.calibration_blocks..spec.calibration_blocks + spec.holdout_blocks,
    )?;
    let holdout = evaluate(&args.out, &spec, population, threshold)?;
    let passed = holdout["passed"] == true;
    evidence::write(root, &args.out, &spec, &frozen, &holdout)?;
    println!(
        "frame-campaign: {} at population {population}, frozen threshold {threshold} percent; holdout {}",
        if passed { "PASS" } else { "FAIL" },
        holdout["holdout_tally"]
    );
    ensure!(
        passed,
        "the holdout did not meet the pre-registered acceptance rule"
    );
    Ok(())
}
