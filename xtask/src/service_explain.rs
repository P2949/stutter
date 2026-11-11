//! A read-only explanation of the installed bounded service: what it is running, what it last did
//! and why it is not tuning, who else owns what, and the exact authority boundary.
//!
//! Observation only. It reads `systemctl show`, `/proc/<pid>/exe`, the service's own state files
//! through the production status loader, the production owner probe and the production authority
//! constants. It builds no second permission engine and issues no request to any worker.
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, ensure};
use clap::Args;
use stutter::api::{
    autotune::{
        AUTONOMOUS_CONTRACTS,
        status::{AutotuneStatus, load_autotune_status},
    },
    daemon_policy::HIGH_RISK_APPLY_IMPLEMENTED,
    ownership::{Daemon, KnownOwner, OwnerReport, SchedExt, probe},
};

use crate::evidence_bundle::sha256::sha256_hex;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct ServiceExplainArgs {
    /// The systemd instance name of the bounded units (the user the service runs for).
    #[arg(long)]
    pub instance: String,
    /// Override the service's state home (default `/var/lib/stutter/autotune-<instance>`).
    #[arg(long)]
    pub state_home: Option<PathBuf>,
    /// New file for the JSON explanation.
    #[arg(long)]
    pub out: PathBuf,
}

fn unit_info(unit: &str) -> serde_json::Value {
    let shown = Command::new("systemctl")
        .args([
            "show",
            "--property=ActiveState,SubState,MainPID,ExecStart",
            unit,
        ])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let field = |name: &str| {
        shown
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
    };
    let pid: Option<u32> = field("MainPID")
        .and_then(|pid| pid.parse().ok())
        .filter(|pid| *pid != 0);
    let exe = pid.and_then(|pid| fs::read_link(format!("/proc/{pid}/exe")).ok());
    let digest = exe
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| sha256_hex(&bytes));
    let configured = field("ExecStart")
        .and_then(|text| {
            text.split("path=")
                .nth(1)
                .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        })
        .filter(|path| path.starts_with("/nix/store/"));
    let configured_sha = configured
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| sha256_hex(&bytes));
    serde_json::json!({
        "unit": unit, "active_state": field("ActiveState"), "sub_state": field("SubState"),
        "main_pid": pid, "executable": exe.map(|p| p.display().to_string()), "executable_sha256": digest,
        "running_executable_readable": digest.is_some(),
        "configured_executable": configured, "configured_executable_sha256": configured_sha,
        "note": "the configured ExecStart store path is what the unit was told to run; the running image is unverified when /proc/<pid>/exe is not readable",
    })
}

pub fn owners_json(report: &OwnerReport) -> serde_json::Value {
    let mut caveats = Vec::new();
    if report.daemon(KnownOwner::ScxLoader) == Daemon::Running
        && !matches!(report.sched_ext, SchedExt::Enabled { .. })
    {
        caveats.push("ScxLoader is likely the permanent kernel thread scx_enable_helper: its comm pattern scx_* also matches kernel threads. No operation maps to the Scheduler domain today, so this hint changes no decision.");
    }
    let running: Vec<String> = report
        .daemons
        .iter()
        .filter(|(_, state)| **state == Daemon::Running)
        .map(|(owner, _)| format!("{owner:?}"))
        .collect();
    serde_json::json!({
        "sched_ext": format!("{:?}", report.sched_ext),
        "daemons_with_a_running_process": running,
        "classification": "ExternalSuspected: a process-name hint only; no exact-task or exact-resource ownership receipt was obtained",
        "caveats": caveats,
    })
}

/// The reasons the service is not tuning right now, each derived from a stated fact.
pub fn why_not_tuned(status: &AutotuneStatus, report: &OwnerReport) -> Vec<String> {
    let mut reasons = Vec::new();
    match &status.target {
        None => reasons.push("no target is selected: no stable, explicitly identified game under scheduler pressure is focused".to_owned()),
        Some(target) => reasons.push(format!("a target is selected ({} pid {}); see phase and last_decision", target.comm, target.pid)),
    }
    reasons.push(format!(
        "controller phase is {:?} in mode {:?}",
        status.phase, status.mode
    ));
    if status.kept_actions.is_empty() {
        reasons.push("no action is currently kept, so there is nothing to restore".to_owned());
    }
    if !status.rollback_available {
        reasons.push("no rollback is pending".to_owned());
    }
    if let Some(fault) = &status.last_fault {
        reasons.push(format!("last fault: {fault}"));
    }
    if let SchedExt::Enabled { ops } = &report.sched_ext {
        reasons.push(format!("sched_ext scheduler {ops} is loaded: nice, uclamp and cgroup weight semantics are unverified, so dependent candidates are denied"));
    }
    if status.data_quality.is_none() {
        reasons.push("objective source, stage and quality are Unknown: no target means no measurement window".to_owned());
    }
    reasons
}

pub fn authority_json() -> serde_json::Value {
    serde_json::json!({
        "autonomous_contracts": AUTONOMOUS_CONTRACTS.iter().map(|c| serde_json::json!({
            "operation": format!("{:?}", c.operation), "workload": format!("{:?}", c.workload),
            "pressure": format!("{:?}", c.pressure), "objective": format!("{:?}", c.objective),
            "effect_scope": format!("{:?}", c.effect_scope),
        })).collect::<Vec<_>>(),
        "high_risk_apply_implemented": HIGH_RISK_APPLY_IMPLEMENTED,
        "note": "nice, uclamp, cpu.weight, PM QoS, GPU and SCX switching are not autonomous operations",
    })
}

pub fn run(args: &ServiceExplainArgs) -> anyhow::Result<()> {
    ensure!(
        !args.instance.is_empty()
            && args.instance.len() <= 32
            && args
                .instance
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
        "the instance must be a plain unit instance name"
    );
    let home = args
        .state_home
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("/var/lib/stutter/autotune-{}", args.instance)));
    ensure!(home.is_dir(), "the state home is not a directory");
    let out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args.out)
        .context("the result file must be new")?;
    let history = home.join(".local/state/stutter/autotune/history.jsonl");
    let status = load_autotune_status(&history).context("load the service status")?;
    let owners = probe(Path::new("/proc"), Path::new("/sys"));
    let value = serde_json::json!({
        "schema_version": 1,
        "evidence_kind": "read_only_service_explanation",
        "controller": unit_info(&format!("stutter-autotune-bounded@{}.service", args.instance)),
        "worker": unit_info(&format!("stutter-privileged-worker@{}.service", args.instance)),
        "controller_status": status,
        "external_owners": owners_json(&owners),
        "authority": authority_json(),
        "why_not_tuned": why_not_tuned(&status, &owners),
        "mutations_issued": 0,
    });
    serde_json::to_writer_pretty(out, &value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stutter::api::ownership::KnownOwner;

    use super::*;

    fn status(target: bool, fault: Option<&str>) -> AutotuneStatus {
        serde_json::from_value(serde_json::json!({
            "phase": "cooldown", "mode": "ApplyLowRisk",
            "target": if target { serde_json::json!({"comm": "game", "pid": 7}) } else { serde_json::Value::Null },
            "focus_group": null, "current_score": null, "active_profile": null, "active_candidate": null,
            "kept_actions": [], "last_decision": "x", "rollback_available": false, "last_rollback_path": null,
            "cooldown_remaining_seconds": null, "planner": null, "data_quality": null, "last_fault": fault,
            "manual_restore_command": "stutter autotune restore", "history_path": "/x",
        })).expect("status")
    }

    fn report(sched_ext: SchedExt) -> OwnerReport {
        OwnerReport {
            sched_ext,
            daemons: BTreeMap::new(),
        }
    }

    #[test]
    fn an_idle_service_is_explained_without_inventing_facts() {
        let reasons = why_not_tuned(&status(false, None), &report(SchedExt::Disabled));
        let text = reasons.join("\n");
        assert!(text.contains("no target is selected"));
        assert!(text.contains("nothing to restore") && text.contains("no rollback is pending"));
        assert!(
            text.contains("Unknown"),
            "a missing measurement is Unknown, not omitted"
        );
        assert!(!text.contains("sched_ext scheduler"));
    }

    #[test]
    fn faults_targets_and_a_foreign_scheduler_are_named() {
        let reasons = why_not_tuned(
            &status(true, Some("worker refused")),
            &report(SchedExt::Enabled {
                ops: "bpfland".into(),
            }),
        );
        let text = reasons.join("\n");
        assert!(text.contains("game pid 7") && text.contains("worker refused"));
        assert!(text.contains("bpfland") && text.contains("unverified"));
    }

    #[test]
    fn the_authority_boundary_is_the_production_constants() {
        let value = authority_json();
        assert_eq!(value["high_risk_apply_implemented"], false);
        let contracts = value["autonomous_contracts"].as_array().expect("array");
        assert_eq!(contracts.len(), AUTONOMOUS_CONTRACTS.len());
        let ops: Vec<&str> = contracts
            .iter()
            .filter_map(|c| c["operation"].as_str())
            .collect();
        assert!(
            ops.iter()
                .all(|op| matches!(*op, "CpuAffinityProfile" | "CgroupPlacement")),
            "{ops:?}"
        );
    }

    #[test]
    fn owners_are_only_ever_suspected() {
        let mut daemons = BTreeMap::new();
        daemons.insert(
            stutter::api::ownership::KnownOwner::GameMode,
            Daemon::Running,
        );
        let value = owners_json(&OwnerReport {
            sched_ext: SchedExt::Disabled,
            daemons,
        });
        assert_eq!(value["daemons_with_a_running_process"][0], "GameMode");
        assert!(
            value["classification"]
                .as_str()
                .is_some_and(|c| c.starts_with("ExternalSuspected"))
        );
    }

    #[test]
    fn a_kernel_thread_scx_hint_carries_its_caveat() {
        let mut daemons = BTreeMap::new();
        daemons.insert(KnownOwner::ScxLoader, Daemon::Running);
        let disabled = owners_json(&OwnerReport {
            sched_ext: SchedExt::Disabled,
            daemons: daemons.clone(),
        });
        assert!(
            disabled["caveats"][0]
                .as_str()
                .is_some_and(|c| c.contains("scx_enable_helper"))
        );
        let enabled = owners_json(&OwnerReport {
            sched_ext: SchedExt::Enabled { ops: "x".into() },
            daemons,
        });
        assert!(
            enabled["caveats"].as_array().is_some_and(Vec::is_empty),
            "a loaded scheduler is not a kernel-thread artefact"
        );
    }

    #[test]
    fn a_bad_instance_or_missing_home_is_refused() {
        let out = std::env::temp_dir().join("service-explain-never-created.json");
        for instance in ["", "../etc", "a b", "UPPER"] {
            let args = ServiceExplainArgs {
                instance: instance.into(),
                state_home: None,
                out: out.clone(),
            };
            assert!(run(&args).is_err(), "{instance:?}");
        }
        let args = ServiceExplainArgs {
            instance: "ok".into(),
            state_home: Some("/definitely/not/here".into()),
            out: out.clone(),
        };
        assert!(run(&args).is_err());
        assert!(!out.exists(), "nothing is written on refusal");
    }
}
