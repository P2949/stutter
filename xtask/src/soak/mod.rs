//! `xtask soak`: the real always-on soak of the graduated operations (PLAN M9, section 12.3).
//!
//! A resumable orchestrator drives the production `stutter autotune` controller and the production
//! privileged worker, as separate processes, through a deterministic schedule of workloads, kept
//! and reverted experiments and injected faults (controller crash and stop, worker death, workload
//! death, suspend/resume), for a wall-clock budget. Everything it concludes is read back from what
//! the controller itself wrote. Durable state lets it resume after a crash or a reboot.
mod history;
mod plan;
mod records;
mod round;
mod verify;
mod world;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::Ordering,
    time::Duration,
};

use anyhow::{Context, ensure};
use clap::{Args, Subcommand};
use records::{Profile, SCHEMA_VERSION, SoakState, append_round, read_state, unix_ms, write_state};

/// The cgroup subtree the soak runs in; the game cgroup is its `target` child.
const DELEGATED: &str = "/stutter-soak/work";
/// Never start a same-operation experiment before the controller's own candidate memory (300 s)
/// would allow it: the soak measures the controller, it does not fight its pacing.
const SAME_OPERATION_SPACING_MS: u64 = 305_000;

const DELEGATE_AND_RUN: &str = r#"
set -eu
root=/sys/fs/cgroup/stutter-soak
echo "+cpu +cpuset" > /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null || true
mkdir -p "$root/work"
echo "+cpu +cpuset" > "$root/cgroup.subtree_control"
chown "$STUTTER_OWNER" "$root" "$root/work"
for f in "$root"/cgroup.procs "$root"/cgroup.threads "$root"/cgroup.subtree_control "$root"/cpuset.* "$root"/cpu.* "$root/work"/cgroup.procs "$root/work"/cgroup.threads; do
  [ -f "$f" ] && chown "$STUTTER_OWNER" "$f" || true
done
echo $$ > "$root/work/cgroup.procs"
uid="${STUTTER_OWNER%%:*}"; gid="${STUTTER_OWNER##*:}"
caps=+bpf,+perfmon,+sys_resource,+dac_read_search,+sys_nice
exec setpriv --reuid "$uid" --regid "$gid" --init-groups --inh-caps=$caps --ambient-caps=$caps -- "$@"
"#;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct SoakArgs {
    #[command(subcommand)]
    pub command: SoakCommand,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum SoakCommand {
    /// Freeze the binaries and create a soak directory.
    Init {
        #[arg(long)]
        dir: PathBuf,
        /// Wall-clock budget in minutes (the real soak is 1440).
        #[arg(long, default_value_t = 1440)]
        minutes: u64,
        #[arg(long, value_enum, default_value_t = Profile::Full)]
        profile: Profile,
        #[arg(long, default_value = "target/debug/stutter")]
        stutter_bin: PathBuf,
        /// Allow a dirty working tree (the soak is then not source-bound; for trial runs only).
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Run (or resume) the soak until its wall-clock budget is spent.
    Run {
        #[arg(long)]
        dir: PathBuf,
    },
    Status {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Check the acceptance list against everything the soak recorded.
    Verify {
        #[arg(long)]
        dir: PathBuf,
    },
}

pub fn run(root: &Path, args: SoakArgs) -> anyhow::Result<()> {
    match args.command {
        SoakCommand::Init {
            dir,
            minutes,
            profile,
            stutter_bin,
            allow_dirty,
        } => init(
            root,
            &dir,
            minutes as f64 / 60.0,
            profile,
            &stutter_bin,
            allow_dirty,
        ),
        SoakCommand::Run { dir } => run_soak(root, &dir),
        SoakCommand::Status { dir } => {
            println!("{}", verify::status_line(&dir)?);
            Ok(())
        }
        SoakCommand::Verify { dir } => {
            let report = verify::verify(&dir)?;
            fs::write(dir.join("verify.json"), serde_json::to_vec_pretty(&report)?)?;
            let text = verify::render(&report);
            fs::write(dir.join("report.txt"), &text)?;
            print!("{text}");
            ensure!(report.passed, "the soak did not meet its acceptance list");
            Ok(())
        }
    }
}

fn sha256(path: &Path) -> anyhow::Result<String> {
    Ok(stutter_core::hash::sha256_hex(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    ))
}

fn git(root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    ensure!(output.status.success(), "git {args:?} failed");
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn pick_cpus() -> anyhow::Result<(u32, u32)> {
    let status = fs::read_to_string("/proc/self/status")?;
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .context("no allowed CPU list")?;
    let mut cpus = Vec::new();
    for part in list.trim().split(',') {
        match part.split_once('-') {
            Some((low, high)) => cpus.extend(low.parse::<u32>()?..=high.parse::<u32>()?),
            None => cpus.push(part.parse()?),
        }
    }
    let siblings = |cpu: u32| -> Vec<u32> {
        fs::read_to_string(format!(
            "/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list"
        ))
        .map(|text| {
            text.trim()
                .split(',')
                .flat_map(|part| match part.split_once('-') {
                    Some((low, high)) => {
                        (low.parse().unwrap_or(0)..=high.parse().unwrap_or(0)).collect::<Vec<_>>()
                    }
                    None => vec![part.parse().unwrap_or(u32::MAX)],
                })
                .collect()
        })
        .unwrap_or_default()
    };
    // Not CPU 0 (interrupts and housekeeping) and not SMT siblings of each other.
    let pool: Vec<u32> = cpus.iter().copied().filter(|cpu| *cpu != 0).collect();
    let first = *pool.get(1).or(pool.first()).context("no CPU available")?;
    let second = pool
        .iter()
        .copied()
        .find(|cpu| *cpu != first && !siblings(first).contains(cpu))
        .context("the allowed CPU set has no second core")?;
    Ok((first, second))
}

fn init(
    root: &Path,
    dir: &Path,
    hours: f64,
    profile: Profile,
    stutter_bin: &Path,
    allow_dirty: bool,
) -> anyhow::Result<()> {
    ensure!(hours > 0.0, "the soak needs a positive duration");
    ensure!(
        !dir.join("soak.json").exists(),
        "{} already holds a soak",
        dir.display()
    );
    if !allow_dirty {
        ensure!(
            git(root, &["status", "--porcelain"])?.is_empty(),
            "the working tree is dirty: a soak is bound to a commit (--allow-dirty for a trial run)"
        );
    }
    let stutter = root.join(stutter_bin);
    ensure!(stutter.is_file(), "{} is not built", stutter.display());
    fs::create_dir_all(dir)?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let state = SoakState {
        schema_version: SCHEMA_VERSION,
        id: format!("soak-{}-{}", &commit[..8], unix_ms()),
        profile,
        seed: unix_ms(),
        hours,
        stutter_bin: stutter.clone(),
        stutter_sha256: sha256(&stutter)?,
        xtask_sha256: sha256(&std::env::current_exe()?)?,
        source_commit: commit,
        created_unix_ms: unix_ms(),
        started_unix_ms: None,
        cpus: Some(pick_cpus()?),
        next_round: 0,
        finished: false,
    };
    write_state(dir, &state)?;
    println!(
        "soak {} initialised in {} ({} h, {:?}, cpus {:?})",
        state.id,
        dir.display(),
        hours,
        profile,
        state.cpus
    );
    Ok(())
}

fn delegated() -> bool {
    fs::read_to_string("/proc/self/cgroup").is_ok_and(|text| text.trim_end().ends_with(DELEGATED))
}

/// Re-run this command inside the delegated cgroup subtree, as the same user with the
/// capabilities a soak needs (never as root, never with a password prompt).
fn reexec_delegated(root: &Path, dir: &Path) -> anyhow::Result<()> {
    let meta = fs::metadata("/proc/self")?;
    let owner = format!(
        "{}:{}",
        std::os::unix::fs::MetadataExt::uid(&meta),
        std::os::unix::fs::MetadataExt::gid(&meta)
    );
    let status = Command::new("sudo")
        .args(["-n", "env"])
        .arg(format!("STUTTER_OWNER={owner}"))
        .args(["sh", "-c", DELEGATE_AND_RUN, "stutter-soak"])
        .arg(std::env::current_exe()?)
        .args(["soak", "run", "--dir"])
        .arg(fs::canonicalize(dir)?)
        .current_dir(root)
        .status()
        .context("sudo -n is required to delegate the cgroup subtree")?;
    ensure!(status.success(), "the soak exited with {status}");
    Ok(())
}

extern "C" fn on_signal(_: i32) {
    round::STOP.store(true, Ordering::SeqCst);
}

fn run_soak(root: &Path, dir: &Path) -> anyhow::Result<()> {
    let mut state = read_state(dir)?;
    ensure!(!state.finished, "this soak is finished; run `soak verify`");
    ensure!(
        sha256(&state.stutter_bin)? == state.stutter_sha256
            && sha256(&std::env::current_exe()?)? == state.xtask_sha256,
        "the binaries changed since `soak init`: a soak is bound to the binaries it started with"
    );
    if !delegated() {
        return reexec_delegated(root, dir);
    }
    world::require_tools()?;
    // SAFETY: the handlers only store to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    let dir = fs::canonicalize(dir)?;
    let cpus = state.cpus.context("the soak has no CPUs")?;
    let mut world = world::World::new(root, &dir, &state.stutter_bin, cpus)?;
    clear_leftovers();
    let started = *state.started_unix_ms.get_or_insert_with(unix_ms);
    write_state(&dir, &state)?;
    let deadline = started + (state.hours * 3_600_000.0) as u64;
    let timing = plan::timing(state.profile);
    let mut last_start = last_starts(&world);
    let mut failures = 0_u32;
    println!(
        "soak {} running: round {}, {:.2} h left",
        state.id,
        state.next_round,
        deadline.saturating_sub(unix_ms()) as f64 / 3_600_000.0
    );
    while unix_ms() < deadline && !round::STOP.load(Ordering::SeqCst) {
        let index = state.next_round;
        let round = plan::round_for(index);
        wait_for_spacing(&round, &last_start);
        if round::STOP.load(Ordering::SeqCst) {
            break;
        }
        let (record, seen) = round::run_round(&mut world, &dir, index, round, timing);
        for event in seen
            .iter()
            .filter(|event| event.decision == "candidate_started")
        {
            if let Some(kind) = &event.action_kind {
                last_start.insert(kind.clone(), unix_ms());
            }
        }
        println!(
            "round {index} {} {} {}: {} ({} violation(s))",
            record.kind,
            record.scenario,
            record.operations,
            record.outcome,
            record.violations.len()
        );
        append_round(&dir, &record)?;
        if record.outcome == "interrupted" {
            break;
        }
        // A round that could not even run (as opposed to one that found a violation) three times
        // in a row means the harness or the host is broken: stop instead of churning.
        failures = if record.error.is_some() {
            failures + 1
        } else {
            0
        };
        anyhow::ensure!(
            failures < 3,
            "three rounds in a row could not run; last error: {}",
            record.error.unwrap_or_default()
        );
        state.next_round += 1;
        write_state(&dir, &state)?;
    }
    world.kill_worker();
    if unix_ms() >= deadline {
        state.finished = true;
        write_state(&dir, &state)?;
        cleanup_cgroup();
        println!(
            "soak {} finished; run `xtask soak verify --dir {}`",
            state.id,
            dir.display()
        );
    } else {
        println!(
            "soak {} paused at round {}; run it again to resume",
            state.id, state.next_round
        );
    }
    Ok(())
}

/// The last start of each graduated operation, from the controller's history (for a resumed soak).
fn last_starts(world: &world::World) -> BTreeMap<String, u64> {
    let mut starts = BTreeMap::new();
    if let Ok(events) = history::read_events(&world.history_path()) {
        for event in events
            .iter()
            .filter(|event| event.decision == "candidate_started")
        {
            if let Some(kind) = &event.action_kind {
                starts.insert(kind.clone(), (event.unix_nanos / 1_000_000) as u64);
            }
        }
    }
    starts
}

/// Idle until every operation the round may use is outside its candidate-memory spacing.
fn wait_for_spacing(round: &plan::Round, last_start: &BTreeMap<String, u64>) {
    let kinds: &[&str] = match round.operations {
        plan::Operations::Affinity => &["cpu_affinity_profile"],
        plan::Operations::Cgroup => &["cgroup_placement"],
        plan::Operations::Both => &["cpu_affinity_profile", "cgroup_placement"],
    };
    let free_at = kinds
        .iter()
        .filter_map(|kind| last_start.get(*kind))
        .map(|at| at + SAME_OPERATION_SPACING_MS)
        .max()
        .unwrap_or(0);
    while unix_ms() < free_at && !round::STOP.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// A previous soak process that died left children in the delegated cgroup: end them.
fn clear_leftovers() {
    let procs =
        fs::read_to_string("/sys/fs/cgroup/stutter-soak/work/cgroup.procs").unwrap_or_default();
    for pid in procs
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
    {
        if pid != std::process::id() {
            world::signal(pid, 9);
        }
    }
}

fn cleanup_cgroup() {
    for dir in ["target", "work", ""] {
        let _ = Command::new("sudo")
            .args(["-n", "rmdir"])
            .arg(Path::new("/sys/fs/cgroup/stutter-soak").join(dir))
            .status();
    }
}
