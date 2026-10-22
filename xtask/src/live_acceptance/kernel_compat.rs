//! Real-kernel compatibility evidence. Compatibility is only ever *observed* from the
//! kernel's own tracepoint formats and a real loader run, never inferred from a distro
//! or kernel name. Reuses the standard acceptance commands and the portable bundle.
use stutter::api::recorder::SessionFile;

use super::{evidence::LiveAcceptanceSuite, *};

/// Scheduler core: without these the profiler cannot observe runnable latency.
pub(super) const REQUIRED: [&str; 2] = ["sched/sched_wakeup", "sched/sched_switch"];
/// Optional evidence and the capability it feeds; absence degrades, never invalidates.
pub(super) const OPTIONAL: [(&str, &str); 6] = [
    ("sched/sched_wakeup_new", "process_lifecycle"),
    ("sched/sched_migrate_task", "migration"),
    ("sched/sched_stat_wait", "sched_stat_wait"),
    ("irq/irq_handler_entry", "irq"),
    ("irq/irq_handler_exit", "irq"),
    ("power/cpu_frequency", "cpu_freq"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Compatibility {
    ObservedCompatible,
    ObservedDegraded,
    ObservedIncompatible,
    Untested,
}
impl Compatibility {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ObservedCompatible => "ObservedCompatible",
            Self::ObservedDegraded => "ObservedDegraded",
            Self::ObservedIncompatible => "ObservedIncompatible",
            Self::Untested => "Untested",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Section {
    pub tracepoint: String,
    pub validation: String,
}

/// Parses `stutter doctor tracepoints --dump` (`--- name (path) [status]` + `validation=`).
pub(super) fn parse_dump(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            let tracepoint = rest.split(" (").next().unwrap_or(rest).to_owned();
            sections.push(Section {
                tracepoint,
                validation: "missing".into(),
            });
        } else if let (Some(value), Some(last)) =
            (line.strip_prefix("validation="), sections.last_mut())
        {
            last.validation = value.trim().to_owned();
        }
    }
    sections
}

/// Optional DRM/KMS evidence from the doctor `kms_timing` check (the existing probe/doctor
/// registry knowledge); absence degrades, it never makes the scheduler core incompatible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DrmKms {
    Usable { provider: String },
    Unavailable(String),
}

/// Block-I/O capability from the production preflight (`tracepoint_formats.details.block_rq`).
/// The raw `block/*` formats in the tracepoint dump are `not_checked` (their variable layout
/// is validated by that preflight), so the dump never decides this capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BlockIo {
    /// Usable; `correlation_basis` is `request-pointer` or the weaker `dev+sector`.
    Usable {
        correlation_basis: String,
    },
    Unavailable(String),
}

fn doctor_check<'a>(report: &'a Value, name: &str) -> Option<&'a Value> {
    report["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|check| check["name"] == name))
}

/// Reads the `kms_timing` check out of `stutter doctor --block-io --kms-timing --json`.
pub(super) fn parse_drm_kms(doctor_json: &str) -> DrmKms {
    let Ok(report) = serde_json::from_str::<Value>(doctor_json) else {
        return DrmKms::Unavailable("doctor output is not valid JSON".into());
    };
    let Some(check) = doctor_check(&report, "kms_timing") else {
        return DrmKms::Unavailable("doctor reported no kms_timing check".into());
    };
    if check["status"] == "Pass" {
        DrmKms::Usable {
            provider: check["details"]["selected_provider"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
        }
    } else {
        DrmKms::Unavailable(
            check["message"]
                .as_str()
                .unwrap_or("kms_timing not usable")
                .to_owned(),
        )
    }
}

/// Reads block-I/O usability from the production tracepoint preflight in the same output.
pub(super) fn parse_block_io(doctor_json: &str) -> BlockIo {
    let Ok(report) = serde_json::from_str::<Value>(doctor_json) else {
        return BlockIo::Unavailable("doctor output is not valid JSON".into());
    };
    let Some(check) = doctor_check(&report, "tracepoint_formats") else {
        return BlockIo::Unavailable("doctor reported no tracepoint_formats check".into());
    };
    let details = &check["details"];
    match details["block_rq"].as_str() {
        Some("ok") => BlockIo::Usable {
            correlation_basis: details["block_io_correlation_basis"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
        },
        other => BlockIo::Unavailable(format!(
            "production block-I/O preflight reports block_rq={}",
            other.unwrap_or("absent")
        )),
    }
}

pub(super) struct Observed {
    pub sections: Vec<Section>,
    pub drm_kms: DrmKms,
    pub block_io: BlockIo,
    /// `Ok` when a real loader run (record + validation) succeeded with zero drops.
    pub loader: Result<(), String>,
}

pub(super) fn classify(observed: &Observed) -> (Compatibility, Value) {
    let ok = |name: &str| {
        observed
            .sections
            .iter()
            .find(|section| section.tracepoint == name)
            // `not_checked` is never treated as validated.
            .is_some_and(|section| section.validation == "compatible")
    };
    let status = |name: &str| -> Value {
        json!({"tracepoint": name, "compatible": ok(name), "validation": observed.sections.iter()
            .find(|s| s.tracepoint == name).map(|s| s.validation.clone())})
    };
    let required = REQUIRED.iter().map(|n| status(n)).collect::<Vec<_>>();
    let optional = OPTIONAL.iter().map(|(n, _)| status(n)).collect::<Vec<_>>();
    let missing_required = REQUIRED
        .iter()
        .filter(|n| !ok(n))
        .copied()
        .collect::<Vec<_>>();
    let mut degraded = std::collections::BTreeSet::new();
    for (name, capability) in OPTIONAL {
        if !ok(name) {
            degraded.insert(capability);
        }
    }
    if matches!(observed.drm_kms, DrmKms::Unavailable(_)) {
        degraded.insert("drm_kms");
    }
    if matches!(observed.block_io, BlockIo::Unavailable(_)) {
        degraded.insert("block_io");
    }
    let mut reasons = Vec::new();
    let state = if observed.sections.is_empty() {
        reasons.push("no tracepoint evidence collected".to_owned());
        Compatibility::Untested
    } else if !missing_required.is_empty() {
        reasons.push(format!(
            "required scheduler tracepoints not compatible: {missing_required:?}"
        ));
        Compatibility::ObservedIncompatible
    } else if let Err(error) = &observed.loader {
        reasons.push(format!("loader/record run failed: {error}"));
        Compatibility::ObservedIncompatible
    } else if !degraded.is_empty() {
        reasons.push("optional evidence unavailable; scheduler core intact".to_owned());
        Compatibility::ObservedDegraded
    } else {
        Compatibility::ObservedCompatible
    };
    (
        state,
        json!({"state":state.as_str(),"required_probes":required,"optional_probes":optional,
            "degraded_capabilities":degraded,"reasons":reasons,
            "loader_result":match &observed.loader { Ok(()) => "ok".to_owned(), Err(error) => error.clone() },
            "drm_kms":match &observed.drm_kms {
                DrmKms::Usable { provider } => json!({"observed":true,"provider":provider}),
                DrmKms::Unavailable(reason) => json!({"observed":false,"reason":reason}),
            },
            "block_io":match &observed.block_io {
                BlockIo::Usable { correlation_basis } => json!({"observed":true,"correlation_basis":correlation_basis}),
                BlockIo::Unavailable(reason) => json!({"observed":false,"reason":reason}),
            },
            "not_covered":["GPU/platform telemetry"]}),
    )
}

pub(super) fn run(root: &Path, out: &Path, binary: &Path) -> anyhow::Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "kernel compatibility requires Linux"
    );
    fs::create_dir(out).context("evidence directory must be new")?;
    let out = fs::canonicalize(out)?;
    let binary = root.join(binary);
    fs::write(
        out.join("acceptance.toml"),
        "# Isolated kernel-compatibility run.\n",
    )?;
    let mut stage = "preflight";
    let mut compatibility = Compatibility::Untested;
    let result = (|| -> anyhow::Result<Value> {
        evidence::metadata(root, &out, &binary, LiveAcceptanceSuite::KernelCompat)?;
        command(
            &binary,
            &args(&["doctor", "tracepoints", "--dump"]),
            &out,
            "tracepoints",
            Duration::from_secs(15),
        )?;
        let dump = fs::read_to_string(out.join("tracepoints.stdout"))?;
        let sections = parse_dump(&dump);
        ensure!(
            sections.iter().any(|s| s.validation != "unavailable"),
            "tracepoint formats are unreadable (permissions?): rerun privileged"
        );
        let (drm_kms, block_io) = optional_run(&binary, &out);
        stage = "loader";
        let loader = loader_run(&binary, &out);
        let (state, report) = classify(&Observed {
            sections,
            drm_kms,
            block_io,
            loader,
        });
        compatibility = state;
        Ok(report)
    })();
    let status = match (&result, compatibility) {
        (Ok(_), Compatibility::ObservedIncompatible) => "FAIL",
        (Ok(_), _) => "PASS",
        (Err(_), _) if stage == "preflight" => "BLOCKED",
        (Err(_), _) => "FAIL",
    };
    let receipt = json!({"schema_version":1,"status":status,"stage":stage,"evidence_kind":"controlled_real",
        "cases":[{"name":compatibility.as_str(),"status":status}],"compatibility":compatibility.as_str(),
        "measurements":result.as_ref().ok(),"error":result.as_ref().err().map(|e| format!("{e:#}"))
            .or_else(|| (compatibility == Compatibility::ObservedIncompatible).then(|| "observed incompatible".to_owned())),
        "scope":"tracepoint-format validation and one real loader/record run on this exact kernel; compatibility is observed, never inferred from distribution or version",
        "not_claimed":["other kernels","GPU/platform telemetry","profiler overhead","portable verified bundle until exported"]});
    evidence::write_result(&out, LiveAcceptanceSuite::KernelCompat, receipt)?;
    println!(
        "{status} ({}): kernel evidence {}",
        compatibility.as_str(),
        out.display()
    );
    result.map(|_| ())
}

/// Optional capabilities from one doctor run (`--block-io --kms-timing`): DRM/KMS from the
/// `kms_timing` check and block I/O from the production tracepoint preflight. A failed or
/// unusable result is `Unavailable` (degraded), never fatal.
fn optional_run(binary: &Path, out: &Path) -> (DrmKms, BlockIo) {
    let unavailable = |reason: String| {
        (
            DrmKms::Unavailable(reason.clone()),
            BlockIo::Unavailable(reason),
        )
    };
    match command(
        binary,
        &args(&["doctor", "--block-io", "--kms-timing", "--json"]),
        out,
        "optional-capabilities",
        Duration::from_secs(15),
    ) {
        Ok(()) => match fs::read_to_string(out.join("optional-capabilities.stdout")) {
            Ok(json) => (parse_drm_kms(&json), parse_block_io(&json)),
            Err(error) => unavailable(format!("{error}")),
        },
        Err(error) => unavailable(format!("{error:#}")),
    }
}

/// One short real record of an owned workload; success needs a valid, drop-free artifact.
fn loader_run(binary: &Path, out: &Path) -> Result<(), String> {
    (|| -> anyhow::Result<()> {
        let ready = out.join("workload-ready.json");
        let mut child = OwnedChild(
            Command::new(std::env::current_exe()?)
                .args(["live-workload", "--seconds", "20", "--ready"])
                .arg(&ready)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .unprivileged()
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            ensure!(
                child.0.try_wait()?.is_none() && Instant::now() < deadline,
                "workload not ready"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let (pid, ticks) = (child.0.id(), start_ticks(child.0.id())?);
        let recordings = out.join("recording");
        let mut record = args(&[
            "record",
            "--tree-pid",
            &pid.to_string(),
            "--duration",
            "5",
            "--summary-ms",
            "250",
            "--no-hwmon",
            "--no-cpu-freq",
            "--no-faults",
            "--no-block-io",
            "--no-stat-wait",
            "--out-dir",
        ]);
        record.push(recordings.to_string_lossy().into_owned());
        command(binary, &record, out, "record", Duration::from_secs(25))?;
        let recording = evidence::recording_dir(&recordings)?;
        evidence::validate(&recording, pid, ticks)?;
        serde_json::from_slice::<SessionFile>(&fs::read(recording.join("session.json"))?)?;
        Ok(())
    })()
    .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(name, validation)| {
                format!("--- {name} (/x/{name}/format) [ok]\nvalidation={validation}\n\n")
            })
            .collect()
    }
    fn all(validation: &'static str) -> Vec<(&'static str, &'static str)> {
        REQUIRED
            .iter()
            .copied()
            .chain(OPTIONAL.iter().map(|(n, _)| *n))
            .map(|n| (n, validation))
            .collect()
    }
    fn drm_present() -> DrmKms {
        DrmKms::Usable {
            provider: "generic_drm".into(),
        }
    }
    fn drm_absent() -> DrmKms {
        DrmKms::Unavailable("no supported pageflip/vblank tracepoints found".into())
    }
    fn block_present(basis: &str) -> BlockIo {
        BlockIo::Usable {
            correlation_basis: basis.into(),
        }
    }
    fn block_absent() -> BlockIo {
        BlockIo::Unavailable("production block-I/O preflight reports block_rq=missing".into())
    }
    fn classified(
        entries: &[(&str, &str)],
        drm_kms: DrmKms,
        block_io: BlockIo,
        loader: Result<(), String>,
    ) -> (Compatibility, Value) {
        classify(&Observed {
            sections: parse_dump(&dump(entries)),
            drm_kms,
            block_io,
            loader,
        })
    }
    fn state(entries: &[(&str, &str)], loader: Result<(), String>) -> Compatibility {
        classified(
            entries,
            drm_present(),
            block_present("request-pointer"),
            loader,
        )
        .0
    }
    /// Complete dump including the raw block formats exactly as `doctor tracepoints --dump`
    /// reports them: read, but `not_checked`.
    fn dump_with_unchecked_block() -> Vec<(&'static str, &'static str)> {
        let mut entries = all("compatible");
        entries.push(("block/block_rq_issue", "not_checked"));
        entries.push(("block/block_rq_complete", "not_checked"));
        entries
    }
    fn doctor_json(kms: &str, block_rq: &str, basis: &str) -> String {
        json!({"checks":[
            {"name":"tracepoint_formats","status":"Pass","message":"",
             "details":{"block_rq":block_rq,"block_io_correlation_basis":basis}},
            {"name":"kms_timing","status":kms,"message":"KMS timing unavailable: none",
             "details":{"selected_provider":"generic_drm"}}]})
        .to_string()
    }

    #[test]
    fn dump_parsing_reads_name_and_validation() {
        let sections = parse_dump(&dump(&[
            ("sched/sched_switch", "compatible"),
            ("irq/irq_handler_entry", "unavailable"),
        ]));
        assert_eq!(
            sections[0],
            Section {
                tracepoint: "sched/sched_switch".into(),
                validation: "compatible".into()
            }
        );
        assert_eq!(sections[1].validation, "unavailable");
    }

    #[test]
    fn four_states_follow_the_required_and_optional_rules() {
        assert_eq!(
            state(&all("compatible"), Ok(())),
            Compatibility::ObservedCompatible
        );
        // Missing optional evidence degrades, never blankets as incompatible.
        let mut degraded = all("compatible");
        degraded.retain(|(n, _)| *n != "sched/sched_stat_wait");
        degraded.push(("sched/sched_stat_wait", "unavailable"));
        assert_eq!(state(&degraded, Ok(())), Compatibility::ObservedDegraded);
        // Required scheduler core missing or mismatched is incompatible.
        let mut broken = all("compatible");
        broken.retain(|(n, _)| *n != "sched/sched_switch");
        broken.push(("sched/sched_switch", "layout_mismatch"));
        assert_eq!(state(&broken, Ok(())), Compatibility::ObservedIncompatible);
        // A real loader failure is incompatibility even with perfect formats.
        assert_eq!(
            state(&all("compatible"), Err("verifier".into())),
            Compatibility::ObservedIncompatible
        );
        // No evidence means Untested, not a guess.
        assert_eq!(state(&[], Ok(())), Compatibility::Untested);
    }

    #[test]
    fn degraded_capabilities_are_named() {
        let mut entries = all("compatible");
        entries.retain(|(n, _)| !n.starts_with("irq/"));
        let (state, report) = classified(&entries, drm_present(), block_absent(), Ok(()));
        assert_eq!(state, Compatibility::ObservedDegraded);
        assert_eq!(report["degraded_capabilities"], json!(["block_io", "irq"]));
    }

    #[test]
    fn readable_but_unchecked_block_formats_do_not_prove_block_io() {
        // The raw block formats are read (`not_checked`) yet the production preflight rejects
        // them: the kernel is degraded for block_io, never compatible.
        let (state, report) = classified(
            &dump_with_unchecked_block(),
            drm_present(),
            block_absent(),
            Ok(()),
        );
        assert_eq!(state, Compatibility::ObservedDegraded);
        assert_eq!(report["degraded_capabilities"], json!(["block_io"]));
        assert_eq!(report["block_io"]["observed"], json!(false));
    }

    #[test]
    fn a_usable_production_block_preflight_makes_block_io_usable() {
        let (state, report) = classified(
            &dump_with_unchecked_block(),
            drm_present(),
            block_present("request-pointer"),
            Ok(()),
        );
        assert_eq!(state, Compatibility::ObservedCompatible);
        assert_eq!(report["degraded_capabilities"], json!([]));
        assert_eq!(report["block_io"]["correlation_basis"], "request-pointer");
    }

    #[test]
    fn the_weaker_dev_sector_fallback_is_usable_and_keeps_its_basis() {
        let (state, report) = classified(
            &all("compatible"),
            drm_present(),
            block_present("dev+sector"),
            Ok(()),
        );
        assert_eq!(state, Compatibility::ObservedCompatible);
        assert_eq!(report["block_io"]["observed"], json!(true));
        assert_eq!(report["block_io"]["correlation_basis"], "dev+sector");
    }

    #[test]
    fn scheduler_core_failure_is_incompatible_regardless_of_optional_state() {
        let mut broken = all("compatible");
        broken.retain(|(n, _)| *n != "sched/sched_wakeup");
        broken.push(("sched/sched_wakeup", "mismatch"));
        for (drm, block) in [
            (drm_present(), block_present("request-pointer")),
            (drm_absent(), block_absent()),
            (drm_present(), block_absent()),
        ] {
            let (state, _) = classified(&broken, drm, block, Ok(()));
            assert_eq!(state, Compatibility::ObservedIncompatible);
        }
    }

    #[test]
    fn drm_present_keeps_a_complete_kernel_compatible() {
        let (state, report) = classified(
            &all("compatible"),
            drm_present(),
            block_present("request-pointer"),
            Ok(()),
        );
        assert_eq!(state, Compatibility::ObservedCompatible);
        assert_eq!(report["drm_kms"]["observed"], json!(true));
        assert_eq!(report["degraded_capabilities"], json!([]));
    }

    #[test]
    fn drm_absence_degrades_and_never_makes_the_kernel_incompatible() {
        let (state, report) = classified(
            &all("compatible"),
            drm_absent(),
            block_present("request-pointer"),
            Ok(()),
        );
        assert_eq!(state, Compatibility::ObservedDegraded);
        assert_eq!(report["degraded_capabilities"], json!(["drm_kms"]));
        assert_eq!(report["drm_kms"]["observed"], json!(false));
    }

    #[test]
    fn doctor_output_maps_to_drm_and_block_observations() {
        let usable = doctor_json("Pass", "ok", "dev+sector");
        assert_eq!(
            parse_drm_kms(&usable),
            DrmKms::Usable {
                provider: "generic_drm".into()
            }
        );
        assert_eq!(parse_block_io(&usable), block_present("dev+sector"));
        let degraded = doctor_json("Warn", "missing", "unavailable");
        assert!(matches!(parse_drm_kms(&degraded), DrmKms::Unavailable(_)));
        assert!(matches!(parse_block_io(&degraded), BlockIo::Unavailable(_)));
        // `not_requested` (probe not asked for) is never usable.
        assert!(matches!(
            parse_block_io(&doctor_json("Pass", "not_requested", "not_requested")),
            BlockIo::Unavailable(_)
        ));
        for garbage in ["not json", "{}"] {
            assert!(matches!(parse_drm_kms(garbage), DrmKms::Unavailable(_)));
            assert!(matches!(parse_block_io(garbage), BlockIo::Unavailable(_)));
        }
    }
}
