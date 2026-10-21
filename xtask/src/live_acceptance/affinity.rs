use super::*;

pub(super) fn run(root: &Path, out: &Path, binary: &Path) -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "affinity acceptance requires Linux"
    );
    fs::create_dir(out).context("affinity evidence directory must be new")?;
    let out = fs::canonicalize(out)?;
    let mut stage = "preflight";
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(root, &out, &root.join(binary), "live_affinity")?;
        let cpu = allowed_cpu()?;
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
        let result = stutter::api::actions::verify_cpu_affinity_lifecycle(
            pid,
            ticks,
            cpu,
            &out.join("restore.json"),
            &out.join("actions.jsonl"),
        )?;
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
    fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"suite":"live_affinity","status":status,"stage":stage,"evidence_kind":"controlled_real",
        "measurements":result.as_ref().ok(),"error":result.as_ref().err().map(|err|format!("{err:#}")),"observation_suite":"not_run","crash_recovery":"not_run"}),
        )?,
    )?;
    println!("{status}: affinity evidence {}", out.display());
    result.map(|_| ())
}
