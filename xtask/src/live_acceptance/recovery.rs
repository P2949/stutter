//! Canonical real SIGKILL/restart recovery, independent of BPF observation.
use stutter::api::autotune::RecoveryCheckpoint;

use super::*;
mod actor;
mod case;
mod procfs;
pub(crate) use actor::{ControllerCommand, controller};

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
        ensure!(
            child.0.try_wait()?.is_none(),
            "owned actor exited before {}; inspect retained stderr",
            path.display()
        );
        if path.is_file() {
            return read_json(path);
        }
        ensure!(
            Instant::now() < deadline,
            "readiness deadline for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn run(root: &Path, out: &Path, binary: &Path) -> anyhow::Result<()> {
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
        evidence::metadata(root, &out, &root.join(binary), "live_recovery")?;
        let cpu = allowed_cpu()?;
        let status = fs::read_to_string("/proc/self/status")?;
        let mask = status
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
            .context("missing harness allowed mask")?
            .trim();
        ensure!(
            mask != cpu.to_string(),
            "recovery mutation acceptance requires at least two allowed CPUs"
        );
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
            cases.push(case::run(&executable, &out, cpu, checkpoint)?);
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
    write_json(
        &out.join("result.json"),
        &json!({"schema_version":1,"suite":"live_recovery","evidence_kind":"controlled_real","status":status,"stage":stage,"cases":cases,
        "error":result.as_ref().err().map(|error|format!("{error:#}")),
        "scope":"production low-risk action transaction and startup recovery; separate real controller processes; fixed owned tree",
        "not_claimed":["full daemon planner/measurement acceptance","interrupted rollback","forced verification/rollback failure physical matrix","PID reuse physical matrix","suspend/resume","provider graduation","portable verified bundle"],
        "observation_suite":"not_run","privacy":"local raw evidence; canonical sanitization/export remains separate"}),
    )?;
    println!("{status}: recovery evidence {}", out.display());
    result
}
