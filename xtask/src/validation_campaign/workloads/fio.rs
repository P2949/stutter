//! `fio` jobs: the fixed-work I/O target and the blocking-storage intervention. Parameters are
//! chosen during uncounted calibration and then frozen; this module only builds commands and
//! summarizes `--output-format=json`.
use std::path::Path;

use serde_json::Value;

use super::CommandSpec;

/// Tunables frozen after calibration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FioProfile {
    pub block_size: String,
    pub file_size: String,
    pub jobs: u32,
}

impl FioProfile {
    /// The steady, modest reader the capture observes ("target").
    pub fn target() -> Self {
        Self {
            block_size: "4k".into(),
            file_size: "64m".into(),
            jobs: 1,
        }
    }

    /// The fixed-work reader of the post-v0.1 study: direct reads, so the device (not the page
    /// cache) decides how long the work takes.
    pub fn target_direct() -> Self {
        Self {
            block_size: "4k".into(),
            file_size: "256m".into(),
            jobs: 1,
        }
    }

    /// Direct, synchronous, fsync'ed writes that are known to occupy the device ("intervention").
    pub fn blocker() -> Self {
        Self {
            block_size: "1m".into(),
            file_size: "1g".into(),
            jobs: 2,
        }
    }
}

fn base(name: &str, directory: &Path, seconds: u64) -> Vec<String> {
    vec![
        format!("--name={name}"),
        format!("--directory={}", directory.display()),
        "--time_based".to_owned(),
        format!("--runtime={seconds}"),
        "--output-format=json".to_owned(),
    ]
}

/// The fixed-work target: a rate-limited random reader, identical in positive and control runs.
pub fn target_command(profile: &FioProfile, directory: &Path, seconds: u64) -> CommandSpec {
    let mut args = base("target", directory, seconds);
    args.extend([
        "--rw=randread".to_owned(),
        format!("--bs={}", profile.block_size),
        format!("--size={}", profile.file_size),
        "--ioengine=psync".to_owned(),
        "--rate_iops=200".to_owned(),
        format!("--numjobs={}", profile.jobs),
        "--group_reporting".to_owned(),
    ]);
    CommandSpec {
        program: "fio".to_owned(),
        args,
        env: Vec::new(),
        cwd: None,
    }
}

/// The post-v0.1 fixed-work target: exactly `ios` direct 4k random reads at queue depth 1. Its
/// elapsed time is the direct effect metric, so it must finish: it is never time-limited.
pub fn fixed_work_command(profile: &FioProfile, directory: &Path, ios: u64) -> CommandSpec {
    CommandSpec {
        program: "fio".to_owned(),
        args: vec![
            "--name=target".to_owned(),
            format!("--directory={}", directory.display()),
            "--output-format=json".to_owned(),
            "--rw=randread".to_owned(),
            format!("--bs={}", profile.block_size),
            format!("--size={}", profile.file_size),
            "--ioengine=psync".to_owned(),
            "--direct=1".to_owned(),
            // Without this fio visits each block once and stops at size/bs I/Os.
            "--norandommap".to_owned(),
            format!("--number_ios={ios}"),
        ],
        env: Vec::new(),
        cwd: None,
    }
}

/// What a finished fixed-work target reports: how long the fixed work took and its latency.
#[derive(Clone, Debug, PartialEq)]
pub struct FixedWorkSummary {
    pub elapsed_ms: f64,
    pub total_ios: u64,
    pub p99_clat_ns: Option<u64>,
}

pub fn fixed_work_summary(json: &str) -> Result<FixedWorkSummary, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("fio output is not JSON: {e}"))?;
    let job = value["jobs"]
        .as_array()
        .and_then(|jobs| jobs.first())
        .ok_or("fio reported no jobs")?;
    if job["error"].as_u64().unwrap_or(0) > 0 {
        return Err(format!("fio reported error {}", job["error"]));
    }
    Ok(FixedWorkSummary {
        elapsed_ms: job["job_runtime"]
            .as_f64()
            .ok_or("fio reported no job_runtime")?,
        total_ios: job["read"]["total_ios"].as_u64().unwrap_or(0),
        p99_clat_ns: job["read"]["clat_ns"]["percentile"]["99.000000"].as_u64(),
    })
}

/// The blocking intervention: bounded, direct, synchronous, fsync'ed writes.
pub fn blocker_command(profile: &FioProfile, directory: &Path, seconds: u64) -> CommandSpec {
    let mut args = base("blocker", directory, seconds);
    args.extend([
        "--rw=write".to_owned(),
        format!("--bs={}", profile.block_size),
        format!("--size={}", profile.file_size),
        "--ioengine=sync".to_owned(),
        "--direct=1".to_owned(),
        "--fsync=1".to_owned(),
        format!("--numjobs={}", profile.jobs),
        "--group_reporting".to_owned(),
    ]);
    CommandSpec {
        program: "fio".to_owned(),
        args,
        env: Vec::new(),
        cwd: None,
    }
}

/// What the independent I/O check needs from a finished job.
#[derive(Clone, Debug, PartialEq)]
pub struct FioSummary {
    pub total_ios: u64,
    pub errors: u64,
    /// 99th percentile completion latency across read and write, nanoseconds.
    pub p99_clat_ns: Option<u64>,
}

pub fn summarize(json: &str) -> Result<FioSummary, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("fio output is not JSON: {e}"))?;
    let jobs = value["jobs"]
        .as_array()
        .filter(|jobs| !jobs.is_empty())
        .ok_or("fio reported no jobs")?;
    let (mut total, mut errors, mut p99): (u64, u64, Option<u64>) = (0, 0, None);
    for job in jobs {
        errors += job["error"].as_u64().unwrap_or(0);
        for direction in ["read", "write"] {
            total += job[direction]["total_ios"].as_u64().unwrap_or(0);
            let latency = job[direction]["clat_ns"]["percentile"]["99.000000"].as_u64();
            p99 = p99.max(latency);
        }
    }
    Ok(FioSummary {
        total_ios: total,
        errors,
        p99_clat_ns: p99,
    })
}

/// A blocking intervention is established only if it completed real work without errors and
/// its writes were slow enough to occupy the device (`min_p99_ns`, frozen at calibration).
pub fn blocking_established(
    summary: &FioSummary,
    min_ios: u64,
    min_p99_ns: u64,
) -> Result<(), String> {
    if summary.errors > 0 {
        return Err(format!("fio reported {} errors", summary.errors));
    }
    if summary.total_ios < min_ios {
        return Err(format!(
            "fio completed {} I/Os, fewer than the required {min_ios}",
            summary.total_ios
        ));
    }
    match summary.p99_clat_ns {
        Some(p99) if p99 >= min_p99_ns => Ok(()),
        other => Err(format!(
            "fio p99 latency {other:?} ns is below the required {min_p99_ns} ns"
        )),
    }
}
