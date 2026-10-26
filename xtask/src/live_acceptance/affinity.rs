use super::*;

pub(super) fn run(
    root: &Path,
    out: &Path,
    binary: &Path,
    mutation: stutter::api::autotune::AcceptanceMutation,
) -> anyhow::Result<()> {
    use stutter::api::autotune::AcceptanceMutation;
    ensure!(
        cfg!(target_os = "linux"),
        "affinity acceptance requires Linux"
    );
    fs::create_dir(out).context("affinity evidence directory must be new")?;
    let out = fs::canonicalize(out)?;
    let mut stage = "preflight";
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(
            root,
            &out,
            &root.join(binary),
            evidence::LiveAcceptanceSuite::Affinity,
        )?;
        let value = match mutation {
            AcceptanceMutation::Affinity => allowed_cpu()?,
            AcceptanceMutation::Uclamp => mutation.default_value(),
        };
        let ready = out.join("workload-ready.json");
        let mut child = OwnedChild(
            Command::new(std::env::current_exe()?)
                .args([
                    "live-workload",
                    "--descendants",
                    "--seconds",
                    "30",
                    "--ready",
                ])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(fs::File::create(out.join("workload.stderr"))?)
                .unprivileged()
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            ensure!(
                child.0.try_wait()?.is_none(),
                "affinity workload exited before readiness"
            );
            ensure!(
                Instant::now() < deadline,
                "affinity workload readiness timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid = child.0.id();
        let ticks = start_ticks(pid)?;
        stage = "affinity_lifecycle";
        let result = match mutation {
            AcceptanceMutation::Affinity => stutter::api::actions::verify_cpu_affinity_lifecycle(
                pid,
                ticks,
                value,
                &out.join("restore.json"),
                &out.join("actions.jsonl"),
            )?,
            AcceptanceMutation::Uclamp => stutter::api::actions::verify_uclamp_lifecycle(
                pid,
                ticks,
                value,
                &out.join("actions.jsonl"),
            )?,
        };
        ensure!(
            child.0.try_wait()?.is_none() && start_ticks(pid)? == ticks,
            "target identity did not survive lifecycle"
        );
        Ok(result)
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
        evidence::LiveAcceptanceSuite::Affinity,
        json!({"schema_version":1,"status":status,"stage":stage,"evidence_kind":"controlled_real","mutation":mutation.as_str(),
        "measurements":result.as_ref().ok(),"error":result.as_ref().err().map(|err|format!("{err:#}")),"observation_suite":"not_run","crash_recovery":"not_run",
        "scope":format!("real multi-process {} apply, physical verify and restore on an owned disposable tree", mutation.as_str()),
        "not_claimed":["controller crash, interrupted rollback and failure matrix (live recovery suite)","suspend/resume","scheduler observation","benefit of the mutation","portable verified bundle until exported"]}),
    )?;
    println!("{status}: affinity evidence {}", out.display());
    result.map(|_| ())
}
