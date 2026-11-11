//! Bounded verification of Cyberpunk's native benchmark output.
//!
//! CD PROJEKT RED documents the `benchmarkResults` directory but not its file schema. This
//! adapter accepts the currently observed `summary.json` metric names and keeps every raw file
//! digest. A real host run must still confirm that schema, scenario and process binding before
//! this evidence can qualify a study.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use stutter_core::ids::Pid;

use super::{CyberpunkInstallation, ProtonSource};

const MAX_RESULT_DIRS: usize = 64;
const MAX_RESULT_FILES: usize = 256;
const MAX_RESULT_DEPTH: usize = 8;
const MAX_RESULT_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RESULT_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_JSON_SUMMARY_BYTES: usize = 4 * 1024 * 1024;
const CLOCK_TOLERANCE_NS: i128 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDirectorySnapshot {
    pub owner_uid: u32,
    pub captured_unix_nanos: u128,
    directories: BTreeMap<String, DirectoryStamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryStamp {
    device: u64,
    inode: u64,
    mtime_seconds: i64,
    mtime_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyberpunkProcessIdentity {
    pub pid: Pid,
    pub start_ticks: u64,
    pub uid: u32,
    pub app_id: u32,
    pub build_id: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBenchmarkMetrics {
    pub average_fps: f64,
    pub minimum_fps: Option<f64>,
    pub maximum_fps: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CyberpunkScenario {
    BuiltinBenchmark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyberpunkBenchmarkAttempt {
    pub installation: CyberpunkInstallation,
    pub process: CyberpunkProcessIdentity,
    pub scene: CyberpunkScenario,
    pub settings_sha256: String,
    pub proton: super::ResearchTool,
    pub started_unix_nanos: u128,
    pub completed_unix_nanos: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBenchmarkFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyberpunkBenchmarkCompletion {
    pub schema_version: u32,
    pub app_id: u32,
    pub build_id: u64,
    pub scene: CyberpunkScenario,
    pub settings_sha256: String,
    pub proton_source: ProtonSource,
    pub proton_identity_sha256: String,
    pub process: CyberpunkProcessIdentity,
    pub started_unix_nanos: u128,
    pub completed_unix_nanos: u128,
    pub result_directory: String,
    pub metrics: NativeBenchmarkMetrics,
    pub files: Vec<NativeBenchmarkFile>,
}

#[derive(Clone, Debug)]
struct ScannedFile {
    evidence: NativeBenchmarkFile,
    bytes: Option<Vec<u8>>,
}

fn now_unix_nanos() -> anyhow::Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && !value
            .bytes()
            .any(|byte| byte == b'/' || byte == 0 || byte.is_ascii_control())
}

fn directory_names(fd: impl AsRawFd) -> anyhow::Result<Vec<String>> {
    let path = format!("/proc/self/fd/{}", fd.as_raw_fd());
    let mut names = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("benchmark output has a non-UTF-8 name"))?;
        anyhow::ensure!(safe_component(&name), "benchmark output has an unsafe name");
        names.push(name);
        anyhow::ensure!(
            names.len() <= MAX_RESULT_FILES,
            "too many benchmark entries"
        );
    }
    names.sort();
    Ok(names)
}

fn directory_stamp(file: &File) -> anyhow::Result<DirectoryStamp> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_dir(),
        "benchmark result entry is not a directory"
    );
    Ok(DirectoryStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        mtime_seconds: metadata.mtime(),
        mtime_nanos: metadata.mtime_nsec(),
    })
}

/// Record the native output folders before launching the game. The path must be the real
/// `benchmarkResults` directory in the selected Steam prefix; this function never creates it.
pub fn snapshot_benchmark_results(
    results_dir: &Path,
    owner_uid: u32,
) -> anyhow::Result<BenchmarkDirectorySnapshot> {
    anyhow::ensure!(owner_uid != 0, "benchmark owner must be unprivileged");
    let root = crate::syscall::open_dir_no_symlinks(results_dir)?;
    let root_file = File::from(root.try_clone()?);
    let root_meta = root_file.metadata()?;
    anyhow::ensure!(
        root_meta.is_dir() && root_meta.uid() == owner_uid,
        "untrusted benchmark results directory"
    );
    let mut directories = BTreeMap::new();
    for name in directory_names(root.as_fd())? {
        let entry_path = format!("/proc/self/fd/{}/{}", root.as_raw_fd(), name);
        let entry_meta = fs::symlink_metadata(&entry_path)?;
        anyhow::ensure!(
            !entry_meta.file_type().is_symlink(),
            "symlink in benchmark results"
        );
        if !entry_meta.is_dir() {
            anyhow::bail!("unexpected non-directory in benchmark results");
        }
        let child = crate::syscall::openat_dir_nofollow(root.as_fd(), &name)?;
        let child = File::from(child);
        let metadata = child.metadata()?;
        anyhow::ensure!(
            metadata.uid() == owner_uid,
            "foreign benchmark result directory"
        );
        directories.insert(name, directory_stamp(&child)?);
        anyhow::ensure!(
            directories.len() <= MAX_RESULT_DIRS,
            "too many benchmark result folders"
        );
    }
    Ok(BenchmarkDirectorySnapshot {
        owner_uid,
        captured_unix_nanos: now_unix_nanos()?,
        directories,
    })
}

fn scan_result_dir(
    fd: impl AsFd,
    owner_uid: u32,
    prefix: &str,
    depth: usize,
    files: &mut Vec<ScannedFile>,
    total_bytes: &mut u64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        depth <= MAX_RESULT_DEPTH,
        "native result directory is too deep"
    );
    let fd = fd.as_fd();
    for name in directory_names(fd)? {
        let path = format!("/proc/self/fd/{}/{}", fd.as_raw_fd(), name);
        let meta = fs::symlink_metadata(&path)?;
        anyhow::ensure!(!meta.file_type().is_symlink(), "symlink in native result");
        let relative = format!("{prefix}/{name}");
        if meta.is_dir() {
            anyhow::ensure!(
                meta.uid() == owner_uid,
                "foreign directory in native result"
            );
            let child = crate::syscall::openat_dir_nofollow(fd, &name)?;
            scan_result_dir(
                child.as_fd(),
                owner_uid,
                &relative,
                depth + 1,
                files,
                total_bytes,
            )?;
            continue;
        }
        anyhow::ensure!(meta.is_file(), "special file in native result");
        anyhow::ensure!(
            meta.uid() == owner_uid && meta.nlink() == 1,
            "untrusted native result file"
        );
        anyhow::ensure!(
            meta.len() <= MAX_RESULT_FILE_BYTES,
            "native result file too large"
        );
        *total_bytes = total_bytes.saturating_add(meta.len());
        anyhow::ensure!(
            *total_bytes <= MAX_RESULT_TOTAL_BYTES,
            "native result exceeds size limit"
        );
        let mut file = crate::syscall::openat_nofollow(fd, &name)?;
        let before = file.metadata()?;
        anyhow::ensure!(
            before.dev() == meta.dev()
                && before.ino() == meta.ino()
                && before.len() == meta.len()
                && before.uid() == owner_uid,
            "native result changed while opening"
        );
        let mut bytes = Vec::with_capacity(before.len() as usize);
        (&mut file)
            .take(MAX_RESULT_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        anyhow::ensure!(
            bytes.len() as u64 == before.len()
                && before.dev() == after.dev()
                && before.ino() == after.ino()
                && before.len() == after.len()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec(),
            "native result changed while reading"
        );
        let summary = relative.rsplit('/').next() == Some("summary.json");
        files.push(ScannedFile {
            evidence: NativeBenchmarkFile {
                path: relative,
                size_bytes: bytes.len() as u64,
                sha256: stutter_core::hash::sha256_hex(&bytes),
            },
            bytes: summary.then_some(bytes),
        });
        anyhow::ensure!(
            files.len() <= MAX_RESULT_FILES,
            "too many native result files"
        );
    }
    Ok(())
}

fn fps_value(value: Option<&serde_json::Value>, name: &str) -> anyhow::Result<Option<f64>> {
    let Some(value) = value else { return Ok(None) };
    let number = value
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("native summary has invalid {name}"))?;
    anyhow::ensure!(
        number.is_finite() && number >= 0.0,
        "native summary has invalid {name}"
    );
    Ok(Some(number))
}

fn parse_summary(bytes: &[u8]) -> anyhow::Result<NativeBenchmarkMetrics> {
    anyhow::ensure!(
        bytes.len() <= MAX_JSON_SUMMARY_BYTES,
        "native summary too large"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("native summary is not a JSON object"))?;
    let average_fps = fps_value(object.get("averageFps"), "averageFps")?
        .filter(|value| *value > 0.0)
        .ok_or_else(|| anyhow::anyhow!("native summary lacks positive averageFps"))?;
    let minimum_fps = fps_value(object.get("minFps"), "minFps")?;
    let maximum_fps = fps_value(object.get("maxFps"), "maxFps")?;
    if let Some(minimum) = minimum_fps {
        anyhow::ensure!(minimum <= average_fps, "native minimum FPS exceeds average");
    }
    if let Some(maximum) = maximum_fps {
        anyhow::ensure!(
            maximum >= average_fps,
            "native maximum FPS is below average"
        );
    }
    Ok(NativeBenchmarkMetrics {
        average_fps,
        minimum_fps,
        maximum_fps,
    })
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Accept exactly one new result folder, verify a nonempty native summary and seal all raw file
/// hashes. The caller separately proves the process-tree, settings and renderer observations.
pub fn verify_benchmark_completion(
    results_dir: &Path,
    before: &BenchmarkDirectorySnapshot,
    attempt: &CyberpunkBenchmarkAttempt,
) -> anyhow::Result<CyberpunkBenchmarkCompletion> {
    let installation = &attempt.installation;
    let process = &attempt.process;
    anyhow::ensure!(before.owner_uid == process.uid, "benchmark owner changed");
    anyhow::ensure!(
        installation.entitlement_verified,
        "Steam entitlement has not been proven by the owned launch"
    );
    anyhow::ensure!(
        installation.manifest_update_status == super::SteamUpdateStatus::NoPendingUpdateIndicated,
        "Steam game update state is not frozen"
    );
    anyhow::ensure!(
        process.pid.as_u32() > 1
            && process.start_ticks > 0
            && process.app_id == installation.app_id
            && process.build_id == installation.build_id
            && process.executable_sha256
                == installation.game_executable_sha256.as_deref().unwrap_or(""),
        "Cyberpunk process identity mismatch"
    );
    anyhow::ensure!(
        attempt.scene == CyberpunkScenario::BuiltinBenchmark,
        "unsupported Cyberpunk scenario"
    );
    anyhow::ensure!(
        digest(&attempt.settings_sha256),
        "invalid settings identity"
    );
    anyhow::ensure!(
        digest(&attempt.proton.identity_sha256),
        "invalid Proton identity"
    );
    anyhow::ensure!(
        before.captured_unix_nanos <= attempt.started_unix_nanos
            && attempt.started_unix_nanos < attempt.completed_unix_nanos
            && attempt.completed_unix_nanos
                <= now_unix_nanos()?.saturating_add(CLOCK_TOLERANCE_NS as u128),
        "invalid benchmark time interval"
    );

    let after = snapshot_benchmark_results(results_dir, process.uid)?;
    let new_dirs: Vec<_> = after
        .directories
        .iter()
        .filter(|(name, _)| !before.directories.contains_key(*name))
        .collect();
    anyhow::ensure!(
        new_dirs.len() == 1,
        "native benchmark did not create exactly one fresh folder"
    );
    let (result_directory, stamp) = new_dirs[0];
    let result_mtime =
        i128::from(stamp.mtime_seconds) * 1_000_000_000 + i128::from(stamp.mtime_nanos);
    let start = i128::try_from(attempt.started_unix_nanos)?;
    let finish = i128::try_from(attempt.completed_unix_nanos)?;
    anyhow::ensure!(
        result_mtime >= start - CLOCK_TOLERANCE_NS && result_mtime <= finish + CLOCK_TOLERANCE_NS,
        "native benchmark result timestamp is outside this attempt"
    );

    let root = crate::syscall::open_dir_no_symlinks(results_dir)?;
    let result = crate::syscall::openat_dir_nofollow(root.as_fd(), result_directory)?;
    let mut files = Vec::new();
    let mut total_bytes = 0;
    scan_result_dir(
        result.as_fd(),
        process.uid,
        result_directory,
        0,
        &mut files,
        &mut total_bytes,
    )?;
    files.sort_by(|left, right| left.evidence.path.cmp(&right.evidence.path));
    let summaries: Vec<_> = files
        .iter()
        .filter_map(|file| file.bytes.as_deref())
        .collect();
    anyhow::ensure!(
        summaries.len() == 1,
        "native result must contain exactly one summary.json"
    );
    anyhow::ensure!(total_bytes > 0, "native result folder is empty");
    let metrics = parse_summary(summaries[0])?;
    Ok(CyberpunkBenchmarkCompletion {
        schema_version: 1,
        app_id: installation.app_id,
        build_id: installation.build_id,
        scene: attempt.scene,
        settings_sha256: attempt.settings_sha256.clone(),
        proton_source: attempt.proton.source,
        proton_identity_sha256: attempt.proton.identity_sha256.clone(),
        process: process.clone(),
        started_unix_nanos: attempt.started_unix_nanos,
        completed_unix_nanos: attempt.completed_unix_nanos,
        result_directory: result_directory.clone(),
        metrics,
        files: files.into_iter().map(|file| file.evidence).collect(),
    })
}

#[cfg(test)]
#[path = "cyberpunk_tests.rs"]
mod tests;
