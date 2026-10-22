//! Real-kernel compatibility evidence. Compatibility is only ever *observed* from the
//! kernel's own tracepoint formats and a real loader run, never inferred from a distro
//! or kernel name. Reuses the standard acceptance commands and the portable bundle.
use stutter::api::recorder::SessionFile;

use super::{evidence::LiveAcceptanceSuite, *};

/// Scheduler core: without these the profiler cannot observe runnable latency.
pub(super) const REQUIRED: [&str; 2] = ["sched/sched_wakeup", "sched/sched_switch"];
/// Optional evidence and the capability it feeds; absence degrades, never invalidates.
pub(super) const OPTIONAL: [(&str, &str); 8] = [
    ("sched/sched_wakeup_new", "process_lifecycle"),
    ("sched/sched_migrate_task", "migration"),
    ("sched/sched_stat_wait", "sched_stat_wait"),
    ("irq/irq_handler_entry", "irq"),
    ("irq/irq_handler_exit", "irq"),
    ("block/block_rq_issue", "block_io"),
    ("block/block_rq_complete", "block_io"),
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

pub(super) struct Observed {
    pub sections: Vec<Section>,
    /// `Ok` when a real loader run (record + validation) succeeded with zero drops.
    pub loader: Result<(), String>,
}

pub(super) fn classify(observed: &Observed) -> (Compatibility, Value) {
    let ok = |name: &str| {
        observed
            .sections
            .iter()
            .find(|section| section.tracepoint == name)
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
            "not_covered":["DRM/KMS","GPU/platform telemetry"]}),
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
        stage = "loader";
        let loader = loader_run(&binary, &out);
        let (state, report) = classify(&Observed { sections, loader });
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
        "not_claimed":["other kernels","DRM/KMS and GPU/platform telemetry","profiler overhead","portable verified bundle until exported"]});
    evidence::write_result(&out, LiveAcceptanceSuite::KernelCompat, receipt)?;
    println!(
        "{status} ({}): kernel evidence {}",
        compatibility.as_str(),
        out.display()
    );
    result.map(|_| ())
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
    fn state(entries: &[(&str, &str)], loader: Result<(), String>) -> Compatibility {
        classify(&Observed {
            sections: parse_dump(&dump(entries)),
            loader,
        })
        .0
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
        entries.retain(|(n, _)| !n.starts_with("irq/") && !n.starts_with("block/"));
        let (state, report) = classify(&Observed {
            sections: parse_dump(&dump(&entries)),
            loader: Ok(()),
        });
        assert_eq!(state, Compatibility::ObservedDegraded);
        assert_eq!(report["degraded_capabilities"], json!(["block_io", "irq"]));
    }
}
