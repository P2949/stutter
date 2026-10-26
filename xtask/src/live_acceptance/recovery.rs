//! Canonical real SIGKILL/restart recovery, independent of BPF observation.
use stutter::api::autotune::{AcceptanceMutation, AcceptanceTarget, RecoveryCheckpoint};

use super::*;
mod actor;
mod case;
mod failure;
mod procfs;
mod suspend;
pub(crate) use actor::{ControllerCommand, controller};

pub(crate) fn parse_mutation(value: &str) -> Result<AcceptanceMutation, String> {
    value
        .parse()
        .map_err(|error: anyhow::Error| error.to_string())
}
pub(crate) use suspend::SuspendOptions;

fn read_json(path: &Path) -> anyhow::Result<Value> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("read {}", path.display()))
}
fn write_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn wait_file(path: &Path, child: &mut OwnedChild, seconds: u64) -> anyhow::Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        // Handshake files are published atomically and a controller may exit right after
        // writing its last one, so the file wins over the liveness check.
        if path.is_file() {
            return read_json(path);
        }
        ensure!(
            child.0.try_wait()?.is_none(),
            "owned actor exited before {}; inspect retained stderr",
            path.display()
        );
        ensure!(
            Instant::now() < deadline,
            "readiness deadline for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn run_suspend(
    root: &Path,
    out: &Path,
    binary: &Path,
    mutation: AcceptanceMutation,
    options: &SuspendOptions,
) -> anyhow::Result<()> {
    suspend::run(root, out, binary, mutation, options)
}

/// The value the mutation applies on this host: a CPU the harness may use, or a `uclamp.min`.
fn target_for(mutation: AcceptanceMutation) -> anyhow::Result<AcceptanceTarget> {
    let value = match mutation {
        AcceptanceMutation::Affinity => allowed_cpu()?,
        AcceptanceMutation::Uclamp | AcceptanceMutation::Nice => mutation.default_value(),
    };
    Ok(AcceptanceTarget { mutation, value })
}

pub(super) fn run(
    root: &Path,
    out: &Path,
    binary: &Path,
    mutation: AcceptanceMutation,
) -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "recovery acceptance requires Linux"
    );
    fs::create_dir(out).context("recovery evidence directory must be new")?;
    let out = fs::canonicalize(out)?;
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated recovery acceptance\n",
    )?;
    let mut stage = "preflight".to_owned();
    let mut cases = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        evidence::metadata(
            root,
            &out,
            &root.join(binary),
            evidence::LiveAcceptanceSuite::Recovery,
        )?;
        let target = target_for(mutation)?;
        if mutation == AcceptanceMutation::Affinity {
            let status = fs::read_to_string("/proc/self/status")?;
            let mask = status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .context("missing harness allowed mask")?
                .trim();
            ensure!(
                mask != target.value.to_string(),
                "recovery mutation acceptance requires at least two allowed CPUs"
            );
        }
        command(
            Path::new("strace"),
            &args(&["--version"]),
            &out,
            "strace-version",
            Duration::from_secs(5),
        )?;
        let executable = std::env::current_exe()?;
        for checkpoint in RecoveryCheckpoint::ALL {
            stage = checkpoint.as_str().to_owned();
            cases.push(if checkpoint.kills_controller() {
                case::run(&executable, &out, target, checkpoint)?
            } else {
                failure::run(&executable, &out, target, checkpoint)?
            });
        }
        ensure!(
            cases.len() == RecoveryCheckpoint::ALL.len(),
            "incomplete recovery checkpoint inventory"
        );
        Ok(())
    })();
    let status = if result.is_ok() {
        "PASS"
    } else if stage == "preflight" {
        "BLOCKED"
    } else {
        "FAIL"
    };
    evidence::write_result(
        &out,
        evidence::LiveAcceptanceSuite::Recovery,
        json!({"schema_version":1,"evidence_kind":"controlled_real","status":status,"stage":stage,"mutation":mutation.as_str(),"cases":cases,
        "error":result.as_ref().err().map(|error|format!("{error:#}")),
        "scope":format!("production {} action transaction (six checkpoints) and one LiveExperimentManager Measuring checkpoint; startup recovery; separate real controller processes; fixed owned tree", mutation.as_str()),
        "not_claimed":["full daemon planner/measurement acceptance (manager and runtime cases use a minimal baseline observation)","replacement-identity physical matrix (covered by startup_recovery real_affinity test)","suspend/resume","provider graduation","portable verified bundle"],
        "observation_suite":"not_run","privacy":"local raw evidence; canonical sanitization/export remains separate"}),
    )?;
    println!("{status}: recovery evidence {}", out.display());
    result
}
