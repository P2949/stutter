use super::*;

pub(super) fn metadata(root: &Path, out: &Path, binary: &Path) -> anyhow::Result<()> {
    let capture = |program: &str, argv: &[&str]| -> anyhow::Result<String> {
        let output = Command::new(program)
            .args(argv)
            .current_dir(root)
            .output()?;
        ensure!(output.status.success(), "metadata command {program} failed");
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    let dirty = !capture("git", &["status", "--porcelain"])?.is_empty();
    let commit = capture("git", &["rev-parse", "HEAD"])?;
    let short = capture("git", &["rev-parse", "--short", "HEAD"])?;
    let version = Command::new(binary).arg("--version").output()?;
    ensure!(version.status.success(), "binary version command failed");
    let version = String::from_utf8(version.stdout)?.trim().to_owned();
    let source_matches_binary =
        version.contains(&format!("git {short})")) || version.contains(&format!("git {commit})"));
    let mut binary_hash_args = vec!["--".to_owned()];
    binary_hash_args.push(binary.to_string_lossy().into_owned());
    // Keep the digest only: absolute checkout and user paths are not provenance.
    let digest = Command::new("sha256sum").args(binary_hash_args).output()?;
    ensure!(digest.status.success(), "binary hash failed");
    let digest = String::from_utf8(digest.stdout)?
        .split_whitespace()
        .next()
        .context("missing digest")?
        .to_owned();
    let os = fs::read_to_string("/etc/os-release")?
        .lines()
        .filter(|line| line.starts_with("ID=") || line.starts_with("VERSION_ID="))
        .collect::<Vec<_>>()
        .join("\n");
    let cpu_model = fs::read_to_string("/proc/cpuinfo")?
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name").and_then(|tail| {
                tail.split_once(':')
                    .map(|(_, value)| value.trim().to_owned())
            })
        });
    let metadata = json!({"schema_version":1,"commit":commit,"binary_version":version,"source_matches_binary":source_matches_binary,
        "source_dirty":dirty,"binary_sha256":digest,"kernel_release":fs::read_to_string("/proc/sys/kernel/osrelease")?.trim(),
        "proc_version":fs::read_to_string("/proc/version")?.trim(),"distribution":os,"cpu_model":cpu_model,
        "cpu_online":fs::read_to_string("/sys/devices/system/cpu/online")?.trim(),
        "rust":capture("rustc", &["-Vv"] )?,"cargo_lock_sha256":capture("sha256sum", &["Cargo.lock"] )?.split_whitespace().next(),
        "evidence_kind":"controlled_real","suite":"live_observation"});
    fs::write(
        out.join("environment.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    ensure!(!dirty, "acceptance requires a clean committed source tree");
    ensure!(
        source_matches_binary,
        "binary revision differs from source; rebuild stutter before acceptance"
    );
    Ok(())
}

pub(super) fn recording_dir(root: &Path) -> anyhow::Result<PathBuf> {
    if root.join("session.json").is_file() {
        return Ok(root.to_owned());
    }
    let candidates = fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("session.json").is_file())
        .collect::<Vec<_>>();
    ensure!(
        candidates.len() == 1,
        "expected exactly one new recording, found {}",
        candidates.len()
    );
    Ok(candidates[0].clone())
}

pub(super) fn validate(root: &Path, pid: u32, ticks: u64) -> anyhow::Result<Value> {
    use stutter::api::recorder::{IntervalRecord, SESSION_SCHEMA_VERSION, SessionFile};
    let session: SessionFile = serde_json::from_slice(&fs::read(root.join("session.json"))?)?;
    let metadata: Value = serde_json::from_slice(&fs::read(root.join("metadata.json"))?)?;
    ensure!(
        session.core.schema_version == SESSION_SCHEMA_VERSION,
        "unexpected artifact schema"
    );
    ensure!(
        metadata["schema_version"].as_u64() == Some(u64::from(SESSION_SCHEMA_VERSION.get())),
        "metadata schema differs"
    );
    ensure!(
        session.core.event_stream_write_errors == 0,
        "recording write errors"
    );
    let drops = serde_json::to_value(&session.core.drop_counters)?;
    ensure!(
        drops
            .as_object()
            .context("drop counters missing")?
            .values()
            .all(|v| v.as_u64() == Some(0)),
        "nonzero or invalid drop counter"
    );
    ensure!(
        session.tasks.iter().any(
            |task| task.process_pid == Some(pid) && task.process_starttime_ticks == Some(ticks)
        ),
        "target process identity missing from session"
    );
    let mut interval_count = 0_u64;
    let mut samples = 0_u64;
    let mut max_p99 = 0_u64;
    let mut classes = std::collections::BTreeSet::new();
    for line in fs::read_to_string(root.join("interval.json"))?
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let interval: IntervalRecord = serde_json::from_str(line)?;
        if interval.process_pid != Some(pid) {
            continue;
        }
        interval_count += 1;
        if interval.samples > 0 {
            ensure!(
                !interval.histogram.is_empty(),
                "direct scheduler histogram missing"
            );
            let histogram_samples = interval
                .histogram
                .iter()
                .try_fold(0_u64, |count, bucket| count.checked_add(bucket.count))
                .context("histogram overflow")?;
            ensure!(
                histogram_samples == interval.samples,
                "incomplete scheduler histogram"
            );
            samples = samples
                .checked_add(interval.samples)
                .context("sample count overflow")?;
            max_p99 = max_p99.max(interval.p99_ns);
            classes.insert(format!("{:?}", interval.class));
        }
    }
    ensure!(
        interval_count > 0 && samples > 0 && max_p99 > 0,
        "no positive target scheduler distribution"
    );
    ensure!(
        session
            .tasks
            .iter()
            .any(|task| task.comm == "accept-wake" && task.process_pid == Some(pid)),
        "periodic wakeup thread not tracked"
    );
    Ok(
        json!({"schema_version":SESSION_SCHEMA_VERSION.get(),"target_pid":pid,"target_starttime_ticks":ticks,
        "target_interval_records":interval_count,"scheduler_samples":samples,"max_interval_p99_ns":max_p99,
        "classes":classes,"drop_counters":drops,"game_objective":"not_claimed_for_generic_acceptance_workload"}),
    )
}

#[cfg(test)]
mod tests {
    use stutter::api::recorder::{
        IntervalRecord, SESSION_SCHEMA_VERSION, SessionFile, SessionTask,
    };

    use super::*;
    #[test]
    fn observation_evidence_requires_identity_samples_histograms_and_zero_drops() {
        let root =
            std::env::temp_dir().join(format!("stutter-live-evidence-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut session = SessionFile::default();
        session.core.schema_version = SESSION_SCHEMA_VERSION;
        session.tasks.push(SessionTask {
            process_pid: Some(42),
            process_starttime_ticks: Some(100),
            comm: "accept-wake".into(),
            ..Default::default()
        });
        fs::write(
            root.join("metadata.json"),
            serde_json::to_vec(&json!({"schema_version":SESSION_SCHEMA_VERSION.get()})).unwrap(),
        )
        .unwrap();
        let mut interval = IntervalRecord {
            process_pid: Some(42),
            samples: 10,
            p99_ns: 1_000,
            ..Default::default()
        };
        let mut value = serde_json::to_value(&interval).unwrap();
        value["histogram"] = json!([{"upper_bound_ns":2_000,"count":10}]);
        interval = serde_json::from_value(value).unwrap();
        let write = |session: &SessionFile, interval: &IntervalRecord| {
            fs::write(
                root.join("session.json"),
                serde_json::to_vec(session).unwrap(),
            )
            .unwrap();
            fs::write(
                root.join("interval.json"),
                serde_json::to_vec(interval).unwrap(),
            )
            .unwrap();
        };
        write(&session, &interval);
        assert!(validate(&root, 42, 100).is_ok());
        assert!(validate(&root, 42, 101).is_err());
        interval.histogram[0].count = 9;
        write(&session, &interval);
        assert!(validate(&root, 42, 100).is_err());
        interval.histogram.clear();
        write(&session, &interval);
        assert!(validate(&root, 42, 100).is_err());
        interval.samples = 0;
        interval.p99_ns = 0;
        write(&session, &interval);
        assert!(validate(&root, 42, 100).is_err());
        session.core.drop_counters.ringbuf_reserve_failed = 1;
        write(&session, &interval);
        assert!(validate(&root, 42, 100).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
