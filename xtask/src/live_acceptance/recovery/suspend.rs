//! Real system suspend/resume acceptance. It suspends the machine, so it only
//! runs with an explicit operator opt-in and is never part of the recovery suite.
use super::{case::start_workload, *};
use crate::live_acceptance::evidence::LiveAcceptanceSuite;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SuspendOptions {
    pub allow_system_suspend: bool,
    pub suspend_command: String,
    pub rtcwake_seconds: Option<u64>,
    pub resume_timeout_seconds: u64,
}

fn blocked(message: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("BLOCKED: {message}")
}
fn suspend_successes() -> Option<u64> {
    fs::read_to_string("/sys/power/suspend_stats/success")
        .ok()?
        .trim()
        .parse()
        .ok()
}
fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

pub(super) fn run(
    root: &Path,
    out: &Path,
    binary: &Path,
    options: &SuspendOptions,
) -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "suspend acceptance requires Linux"
    );
    fs::create_dir(out).context("suspend evidence directory must be new")?;
    let out = fs::canonicalize(out)?;
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated suspend/resume acceptance\n",
    )?;
    let mut cases = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        if !options.allow_system_suspend {
            return Err(blocked(
                "this suite suspends the machine; pass --allow-system-suspend to opt in",
            ));
        }
        evidence::metadata(
            root,
            &out,
            &root.join(binary),
            LiveAcceptanceSuite::SuspendResume,
        )?;
        let cpu = allowed_cpu()?;
        cases.push(case(&std::env::current_exe()?, &out, cpu, options)?);
        Ok(())
    })();
    let blocked_error = result
        .as_ref()
        .err()
        .is_some_and(|error| format!("{error:#}").starts_with("BLOCKED:"));
    let status = match (&result, blocked_error) {
        (Ok(()), _) => "PASS",
        (Err(_), true) => "BLOCKED",
        (Err(_), false) => "FAIL",
    };
    evidence::write_result(
        &out,
        LiveAcceptanceSuite::SuspendResume,
        json!({"schema_version":1,"evidence_kind":"controlled_real","status":status,"cases":cases,
        "error":result.as_ref().err().map(|error|format!("{error:#}")),
        "scope":"one real system suspend/resume with an active low-risk CPU-affinity candidate; production lifecycle policy in AutotuneRuntime",
        "not_claimed":["suspend during apply/verify/rollback","multiple suspend cycles","planner/measurement behavior after stabilization","portable verified bundle"],
        "privacy":"local raw evidence; canonical sanitization/export remains separate"}),
    )?;
    println!("{status}: suspend/resume evidence {}", out.display());
    result
}

fn case(
    executable: &Path,
    out: &Path,
    cpu: u32,
    options: &SuspendOptions,
) -> anyhow::Result<Value> {
    let mut workload = start_workload(
        executable,
        out,
        options.resume_timeout_seconds.saturating_add(180),
    )?;
    let root = workload.0.id();
    let root_ticks = start_ticks(root)?;
    let before = procfs::snapshot(root, root_ticks)?;
    let desired = cpu.to_string();
    ensure!(
        before.values().all(|task| task.mask != desired),
        "no effective mask change for owned helper"
    );
    let result = (|| -> anyhow::Result<Value> {
        write_json(&out.join("before.json"), &procfs::value(&before))?;
        let mut controller = OwnedChild(
            Command::new(executable)
                .args(["recovery-controller", "suspend-resume", "--out"])
                .arg(out)
                .args([
                    "--tree-pid",
                    &root.to_string(),
                    "--tree-ticks",
                    &root_ticks.to_string(),
                    "--cpu",
                    &cpu.to_string(),
                    "--owner-pid",
                    &std::process::id().to_string(),
                    "--owner-ticks",
                    &start_ticks(std::process::id())?.to_string(),
                    "--resume-timeout-seconds",
                    &options.resume_timeout_seconds.to_string(),
                ])
                .env("STUTTER_CONFIG", out.join("acceptance.toml"))
                .stdout(fs::File::create(out.join("controller.stdout"))?)
                .stderr(fs::File::create(out.join("controller.stderr"))?)
                .spawn()?,
        );
        wait_file(&out.join("controller-started.json"), &mut controller, 10)?;
        let ready = wait_file(&out.join("suspend-ready.json"), &mut controller, 20)?;
        ensure!(
            ready["journal_state"] == "measuring"
                && ready["rollback_token_present"] == true
                && ready["active_experiment"] == true
                && ready["window_intervals"] == 1,
            "pre-suspend controller state differs: {ready}"
        );
        let at_ready = procfs::snapshot(root, root_ticks)?;
        ensure!(
            procfs::changed(&before, &at_ready, &desired)? == 5,
            "candidate is not physically applied before suspend"
        );
        write_json(&out.join("at-suspend.json"), &procfs::value(&at_ready))?;
        let suspends_before = suspend_successes();
        let pre = json!({"unix_ms":unix_ms().to_string(),"suspend_stats_success":suspends_before,"controller":ready});
        write_json(&out.join("pre-suspend-receipt.json"), &pre)?;
        if let Some(seconds) = options.rtcwake_seconds {
            command(
                Path::new("rtcwake"),
                &args(&["-m", "no", "-s", &seconds.to_string()]),
                out,
                "rtcwake-arm",
                Duration::from_secs(10),
            )
            .map_err(|error| blocked(format!("could not arm RTC wake alarm: {error:#}")))?;
        }
        let mut words = options.suspend_command.split_whitespace();
        let program = words.next().context("empty suspend command")?;
        let words = words.map(str::to_owned).collect::<Vec<_>>();
        command(
            Path::new(program),
            &words,
            out,
            "suspend",
            Duration::from_secs(options.resume_timeout_seconds),
        )
        .map_err(|error| {
            blocked(format!(
                "suspend command failed or never resumed: {error:#}"
            ))
        })?;
        let handled = wait_file(&out.join("resume-handled.json"), &mut controller, 120)?;
        let status = controller.0.wait()?;
        ensure!(status.success(), "controller exited with {status}");
        let slept_ms: u128 = handled["slept_ms"]
            .as_str()
            .and_then(|value| value.parse().ok())
            .context("slept_ms missing")?;
        ensure!(
            slept_ms >= 5_000,
            "suspended interval too short: {slept_ms}"
        );
        let actions = handled["actions"]
            .as_array()
            .context("actions missing")?
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        for required in [
            "clear_measurement_windows",
            "wait_stabilization",
            "refresh_target_identity",
        ] {
            ensure!(
                actions.contains(&required),
                "resume policy lacks {required}"
            );
        }
        ensure!(
            handled["window_intervals"] == 0 && handled["active_experiment"] == false,
            "pre-suspend measurement or candidate survived resume: {handled}"
        );
        ensure!(
            matches!(
                handled["journal_state"].as_str(),
                Some("reverted" | "clean")
            ),
            "journal did not reach a terminal state: {handled}"
        );
        ensure!(
            handled["cooldown_remaining_ms"]
                .as_str()
                .and_then(|value| value.parse::<u128>().ok())
                .is_some_and(|ms| ms > 0),
            "no stabilization cooldown after resume"
        );
        let restored = procfs::snapshot(root, root_ticks)?;
        ensure!(
            restored == before,
            "independent readback differs after resume"
        );
        write_json(&out.join("restored.json"), &procfs::value(&restored))?;
        let suspends_after = suspend_successes();
        if let (Some(before), Some(after)) = (suspends_before, suspends_after) {
            ensure!(
                after > before,
                "kernel suspend_stats did not record a successful suspend"
            );
        }
        Ok(
            json!({"status":"PASS","processes":3,"tasks":5,"pre_suspend":pre,"resume":handled,
            "kernel_suspend_stats":{"before":suspends_before,"after":suspends_after},
            "independent_restoration":true}),
        )
    })();
    let _ = workload.0.kill();
    let root_reaped = workload.0.wait().is_ok();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !procfs::stopped(&before) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let cleanup = procfs::stopped(&before);
    write_json(
        &out.join("cleanup.json"),
        &json!({"root_reaped":root_reaped,"all_owned_task_identities_ceased_running":cleanup}),
    )?;
    let mut result = result?;
    ensure!(
        cleanup && root_reaped,
        "owned helper cleanup/reaping failed"
    );
    result["cleanup_verified"] = json!(true);
    Ok(result)
}
