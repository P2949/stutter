//! One measured run: a pinned `vkcube` under MangoHud with one busy loop at a fixed nice value on
//! the same CPU. The frame log is read with the controller's own parser and rolling window.
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, ensure};
use serde_json::to_value;
use stutter::api::autotune::frame_log_stats;

use super::{FrameRun, Spec, block_levels, run_path};
use crate::objective_campaign::{stats::Condition, write_json};

fn spawn_hog(cpu: u32, nice: i32) -> anyhow::Result<Child> {
    Command::new("nice")
        .args(["-n", &nice.to_string(), "taskset", "-c", &cpu.to_string()])
        .args(["sh", "-c", "while :; do :; done"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start an interference process")
}

fn spawn_workload(cpu: u32, seconds: u64, log_dir: &Path) -> anyhow::Result<Child> {
    Command::new("taskset")
        .args(["-c", &cpu.to_string(), "mangohud", "vkcube"])
        .args(["--c", "2000000000", "--width", "640", "--height", "480"])
        .args(["--present_mode", "0"])
        .env(
            "MANGOHUD_CONFIG",
            format!(
                "autostart_log=1,log_duration={seconds},log_interval=0,output_folder={}",
                log_dir.display()
            ),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start vkcube under MangoHud")
}

/// The detailed MangoHud log (not the summary) written into `dir`.
fn detailed_log(dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    let mut found = None;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".csv") && !name.contains("summary") {
            found = Some(path);
        }
    }
    found.with_context(|| format!("MangoHud wrote no frame log into {}", dir.display()))
}

pub(crate) fn execute_run(
    out: &Path,
    spec: &Spec,
    block: u32,
    position: u32,
    condition: Condition,
) -> anyhow::Result<()> {
    let (light, heavy) = block_levels(spec, block);
    let nice = if condition == Condition::Clean {
        light
    } else {
        heavy
    };
    let log_dir = out.join("logs").join(format!("{block:03}-{position}"));
    fs::create_dir_all(&log_dir)?;
    let mut hog = spawn_hog(spec.cpu, nice)?;
    let outcome = (|| {
        let mut workload = spawn_workload(spec.cpu, spec.seconds, &log_dir)?;
        // The log is complete once `log_duration` has elapsed; the margin covers startup and flush.
        thread::sleep(Duration::from_secs(spec.seconds + 5));
        let _ = workload.kill();
        let _ = workload.wait();
        anyhow::Ok(())
    })();
    let _ = hog.kill();
    let _ = hog.wait();
    outcome?;

    let log = detailed_log(&log_dir)?;
    let max_population = *spec
        .population_grid
        .last()
        .context("empty population grid")?;
    let mut p99_us = std::collections::BTreeMap::new();
    let mut valid_frames = 0;
    for &population in &spec.population_grid {
        let stats = frame_log_stats(&log, Some(population))?;
        ensure!(
            stats.valid_frames >= population,
            "run {block}-{position} has only {} valid frames, below the population {population}",
            stats.valid_frames
        );
        ensure!(stats.p99_ms > 0.0, "frame p99 must be positive");
        p99_us.insert(population, (stats.p99_ms * 1000.0).round().max(1.0) as u64);
    }
    valid_frames = valid_frames.max(frame_log_stats(&log, None)?.valid_frames);
    ensure!(valid_frames >= max_population, "too few frames in the log");
    // Only the first `max_population` frames decide anything; keep that much of the log.
    let trimmed = log_dir.join("frames-trimmed.csv");
    let text = fs::read_to_string(&log)?;
    let header_lines = 3;
    fs::write(
        &trimmed,
        text.lines()
            .take(header_lines + max_population + 50)
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    fs::remove_file(&log)?;
    let run = FrameRun {
        block,
        position,
        condition,
        nice,
        valid_frames,
        p99_us,
        execution_id: format!("frame-{block:03}-{position}"),
    };
    write_json(&run_path(out, block, position), &to_value(&run)?)?;
    println!(
        "run {block:03}-{position}: nice {nice}, {valid_frames} frames, p99 {} us at {max_population}",
        run.p99_us[&max_population]
    );
    Ok(())
}
