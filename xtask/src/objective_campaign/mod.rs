//! Calibrating and graduating an objective's decision rule from measured pairs.
//!
//! The parameters are pre-registered in `docs/validation/objective-graduation.toml`. Calibration
//! pairs choose a threshold, which is written to disk (frozen) before any holdout run exists; the
//! untouched holdout is then evaluated once and never recomputed. Every run is its own file, so an
//! interrupted campaign resumes without repeating or reordering what it already measured.
mod stats;
#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use anyhow::{Context, bail, ensure};
use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};
use stats::{Condition, Pair, Rule, Run, Tally, holdout_passes, pairs, select_threshold, tally};

use crate::evidence_metadata::{self as shared, SUITE_COMPILE_OBJECTIVE, Source};

pub const MANIFEST: &str = "docs/validation/objective-graduation.toml";

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
    compile_throughput: Spec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    objective: String,
    package: String,
    cpus: String,
    jobs: u32,
    interference_cpus: Vec<u32>,
    calibration_blocks: u32,
    holdout_blocks: u32,
    threshold_grid_percent: Vec<f64>,
    noise_margin: f64,
    min_beneficial_keep_rate: f64,
    max_false_keeps: u32,
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

fn parse_manifest(text: &str) -> anyhow::Result<Spec> {
    let file: File = toml::from_str(text)?;
    ensure!(
        file.schema_version == 1,
        "unsupported objective campaign schema"
    );
    let spec = file.compile_throughput;
    ensure!(
        spec.objective == "CompileThroughputWithForegroundProtection",
        "the manifest does not describe the compile throughput objective"
    );
    ensure!(
        spec.calibration_blocks >= 10 && spec.holdout_blocks >= 5,
        "a campaign needs at least 10 calibration and 5 holdout blocks"
    );
    ensure!(
        !spec.threshold_grid_percent.is_empty()
            && spec
                .threshold_grid_percent
                .iter()
                .all(|value| *value > 0.0 && value.is_finite()),
        "the threshold grid must be positive and non-empty"
    );
    ensure!(
        spec.noise_margin >= 1.0,
        "the noise margin cannot be below 1"
    );
    ensure!(
        (0.8..=1.0).contains(&spec.min_beneficial_keep_rate),
        "the beneficial keep rate cannot be loosened below 0.8"
    );
    ensure!(
        spec.max_false_keeps == 0,
        "a false keep is never acceptable"
    );
    ensure!(
        spec.interference_cpus.len() >= 2 && spec.jobs > 0,
        "the campaign needs at least two interference CPUs and a positive job count"
    );
    Ok(spec)
}

/// The conditions of a block in run order. The order alternates so that neither direction of a
/// comparison is always measured first.
fn block_order(block: u32) -> [Condition; 4] {
    use Condition::{Clean as C, Interfered as I};
    if block.is_multiple_of(2) {
        [C, C, I, C]
    } else {
        [C, I, C, C]
    }
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run_path(out: &Path, block: u32, position: u32) -> PathBuf {
    out.join("runs").join(format!("{block:03}-{position}.json"))
}

fn load_runs(out: &Path, blocks: std::ops::Range<u32>) -> anyhow::Result<Vec<Run>> {
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

fn spawn_hog(cpu: u32) -> anyhow::Result<Child> {
    Command::new("taskset")
        .args(["-c", &cpu.to_string(), "sh", "-c", "while :; do :; done"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start an interference process")
}

/// One cold build timed by the fixed-work runner, with interference when the condition says so.
fn execute_run(
    root: &Path,
    out: &Path,
    spec: &Spec,
    tree: &str,
    block: u32,
    position: u32,
    condition: Condition,
) -> anyhow::Result<Run> {
    let target = std::env::temp_dir().join("stutter-objective-campaign-target");
    let _ = fs::remove_dir_all(&target);
    let work = out.join("work");
    fs::create_dir_all(&work)?;
    let stem = format!("{block:03}-{position}");
    let markers = work.join(format!("{stem}.markers"));
    fs::write(&markers, b"")?;
    let identity = work.join(format!("{stem}.identity.json"));
    write_json(
        &identity,
        &json!({
            "kind": "compile",
            "source_tree": format!("git:{tree}"),
            "target": format!("{} debug build, cold target directory", spec.package),
            "build_state": "cold: target directory removed before every run",
            "cache_policy": "offline, private target directory, no shared compiler cache",
            "job_count": spec.jobs,
            "command": [
                "taskset", "-c", spec.cpus, "cargo", "build", "--offline", "-q",
                "-p", spec.package, "-j", spec.jobs.to_string(),
                "--target-dir", target.to_string_lossy(),
            ],
            "working_directory": root.to_string_lossy(),
            "toolchain": "the pinned nightly of the development shell",
            "environment": format!("development shell, CPUs {}", spec.cpus),
        }),
    )?;
    let mut hogs = Vec::new();
    if condition == Condition::Interfered {
        for cpu in spec.interference_cpus.iter().take(2) {
            hogs.push(spawn_hog(*cpu)?);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let status = Command::new(std::env::current_exe()?)
        .arg("fixed-work")
        .arg("--identity")
        .arg(&identity)
        .arg("--markers")
        .arg(&markers)
        .current_dir(root)
        .output();
    for hog in &mut hogs {
        let _ = hog.kill();
        let _ = hog.wait();
    }
    let status = status.context("run the fixed-work runner")?;
    ensure!(
        status.status.success(),
        "the fixed-work runner failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let complete = fs::read_to_string(&markers)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|marker| marker["phase"] == "WORKLOAD_COMPLETE")
        .context("the run produced no completion marker")?;
    let run = Run {
        block,
        position,
        condition,
        elapsed_ns: complete["elapsed_ns"].as_u64().context("no elapsed time")?,
        execution_id: complete["execution_id"]
            .as_str()
            .context("no execution id")?
            .to_owned(),
    };
    write_json(
        &run_path(out, block, position),
        &serde_json::to_value(&run)?,
    )?;
    println!(
        "block {block} run {position} {condition:?}: {:.3} s",
        run.elapsed_ns as f64 / 1e9
    );
    Ok(run)
}

fn collect(
    root: &Path,
    out: &Path,
    spec: &Spec,
    tree: &str,
    blocks: std::ops::Range<u32>,
) -> anyhow::Result<()> {
    for block in blocks {
        for (position, condition) in block_order(block).into_iter().enumerate() {
            let position = position as u32;
            if !run_path(out, block, position).exists() {
                execute_run(root, out, spec, tree, block, position, condition)?;
            }
        }
    }
    Ok(())
}

/// Selects the threshold on calibration pairs and writes it down. Refuses to run once any holdout
/// run exists without a frozen threshold, so the holdout can never inform the selection.
fn freeze(out: &Path, spec: &Spec) -> anyhow::Result<Value> {
    let frozen = out.join("frozen.json");
    if frozen.exists() {
        return read_json(&frozen);
    }
    let first_holdout = spec.calibration_blocks;
    ensure!(
        !run_path(out, first_holdout, 0).exists(),
        "holdout data exists but no threshold was frozen; the holdout is contaminated"
    );
    let calibration = pairs(&load_runs(out, 0..spec.calibration_blocks)?);
    let Some((threshold, tally)) =
        select_threshold(&calibration, &spec.threshold_grid_percent, &spec.rule())
    else {
        write_json(
            &out.join("unproven.json"),
            &json!({"reason": "no pre-registered threshold satisfies the selection rule on calibration", "calibration_pairs": calibration}),
        )?;
        bail!(
            "no pre-registered threshold satisfies the rule: the objective stays unproven (DeferredEvidence)"
        );
    };
    let value = json!({
        "threshold_percent": threshold,
        "calibration_tally": tally,
        "calibration_keep_rate": tally.keep_rate(),
        "neutral_noise_percent": stats::neutral_noise_percent(&calibration),
        "calibration_pairs": calibration,
        "rule": {"noise_margin": spec.noise_margin, "min_beneficial_keep_rate": spec.min_beneficial_keep_rate, "max_false_keeps": spec.max_false_keeps},
    });
    write_json(&frozen, &value)?;
    println!("frozen threshold: {threshold} percent");
    Ok(value)
}

/// The single holdout evaluation. An existing result is returned as it is and never recomputed.
fn evaluate(out: &Path, spec: &Spec, threshold: f64) -> anyhow::Result<Value> {
    let path = out.join("holdout.json");
    if path.exists() {
        return read_json(&path);
    }
    let range = spec.calibration_blocks..spec.calibration_blocks + spec.holdout_blocks;
    let holdout: Vec<Pair> = pairs(&load_runs(out, range)?);
    let result: Tally = tally(&holdout, threshold);
    let passed = holdout_passes(&result, &spec.rule());
    let value = json!({
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
        "an objective campaign needs a clean committed source tree"
    );
    let spec = parse_manifest(&fs::read_to_string(root.join(MANIFEST))?)?;
    let git = |argv: &[&str]| -> anyhow::Result<String> {
        let output = Command::new("git").args(argv).current_dir(root).output()?;
        ensure!(output.status.success(), "git {} failed", argv.join(" "));
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    let (commit, tree) = (
        git(&["rev-parse", "HEAD"])?,
        git(&["rev-parse", "HEAD^{tree}"])?,
    );
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
    collect(root, &args.out, &spec, &tree, 0..spec.calibration_blocks)?;
    let frozen = freeze(&args.out, &spec)?;
    let threshold = frozen["threshold_percent"]
        .as_f64()
        .context("no frozen threshold")?;
    collect(
        root,
        &args.out,
        &spec,
        &tree,
        spec.calibration_blocks..spec.calibration_blocks + spec.holdout_blocks,
    )?;
    let holdout = evaluate(&args.out, &spec, threshold)?;
    let passed = holdout["passed"] == true;
    write_evidence(root, &args.out, &spec, &frozen, &holdout)?;
    println!(
        "objective-campaign: {} at a frozen threshold of {threshold} percent; holdout {}",
        if passed { "PASS" } else { "FAIL" },
        holdout["holdout_tally"]
    );
    ensure!(
        passed,
        "the holdout did not meet the pre-registered acceptance rule"
    );
    Ok(())
}

fn write_evidence(
    root: &Path,
    out: &Path,
    spec: &Spec,
    frozen: &Value,
    holdout: &Value,
) -> anyhow::Result<()> {
    let raw = out.join("evidence-raw");
    if raw.exists() {
        fs::remove_dir_all(&raw)?;
    }
    fs::create_dir_all(raw.join("artifacts"))?;
    let environment = shared::capture(
        root,
        &root.join("target/debug/stutter"),
        SUITE_COMPILE_OBJECTIVE,
        "controlled_real",
        Source::Checkout,
    )?;
    shared::write_environment(&raw, &environment)?;
    shared::require_clean(&environment)?;
    write_json(&raw.join("artifacts/frozen-calibration.json"), frozen)?;
    write_json(&raw.join("artifacts/holdout.json"), holdout)?;
    let all_runs = load_runs(out, 0..spec.calibration_blocks + spec.holdout_blocks)?;
    write_json(
        &raw.join("artifacts/runs.json"),
        &serde_json::to_value(&all_runs)?,
    )?;
    let passed = holdout["passed"] == true;
    let tally = &holdout["holdout_tally"];
    let case = |name: &str, ok: bool, detail: String| json!({"name": name, "status": if ok { "PASS" } else { "FAIL" }, "detail": detail});
    shared::write_result(
        &raw,
        SUITE_COMPILE_OBJECTIVE,
        json!({
            "status": if passed { "PASS" } else { "FAIL" },
            "evidence_kind": "controlled_real",
            "scope": "Direct elapsed-time evidence of identical completed cold builds of one package, timed by the fixed-work runner, separates a controlled beneficial change from a neutral one and a regression: the decision threshold was selected by a pre-registered rule on calibration pairs, frozen before any holdout run existed, and evaluated once on an untouched holdout with no false keep.",
            "not_claimed": [
                "any mutation's benefit (this graduates the objective's measurement, not an operation)",
                "foreground-protection or memory-pressure guardrail behavior under live load",
                "workloads other than a cold build of one package on four CPUs",
                "real user sessions",
            ],
            "cases": [
                case("threshold_selected_by_the_preregistered_rule_on_calibration", true,
                     format!("{} percent", frozen["threshold_percent"])),
                case("no_false_keep_on_the_holdout", tally["false_keeps"] == 0,
                     format!("{} false keeps over {} neutral and {} regression pairs", tally["false_keeps"], tally["neutral"], tally["regression"])),
                case("beneficial_keep_rate_on_the_holdout", holdout["holdout_keep_rate"].as_f64().unwrap_or(0.0) >= spec.min_beneficial_keep_rate,
                     format!("{} of {} beneficial pairs kept", tally["beneficial_kept"], tally["beneficial"])),
            ],
        }),
    )?;
    Ok(())
}
