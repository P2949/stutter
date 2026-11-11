//! Applies the planner's REAL owner-conflict checks to the LIVE host's sched_ext state.
//!
//! The deployed service never proposes the refused knobs, so this does not run the service: it
//! probes the kernel's present sched_ext owner with the production `probe`, then asks the production
//! `knob_is_honored` and `owner_denial` what each candidate kind would get. Observation only.
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::Args;
use stutter::api::{
    autotune::MutationOperationKind as Op,
    ownership::{
        SchedExt, owner_denial, probe,
        sched_semantics::{Knob, knob_is_honored},
    },
};

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct OwnerRefusalArgs {
    /// New file for the JSON result.
    #[arg(long)]
    pub out: PathBuf,
}

pub fn run(args: &OwnerRefusalArgs) -> anyhow::Result<()> {
    let out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args.out)
        .context("the result file must be new")?;
    let report = probe(Path::new("/proc"), Path::new("/sys"));
    let ops = match &report.sched_ext {
        SchedExt::Enabled { ops } => Some(ops.as_str()),
        _ => None,
    };
    let knobs = [
        ("nice", Knob::Nice),
        ("uclamp", Knob::Uclamp),
        ("cgroup_cpu_weight", Knob::CgroupCpuWeight),
    ];
    let operations = [("Nice", Op::Nice), ("Uclamp", Op::Uclamp)];
    let value = serde_json::json!({
        "schema_version": 1,
        "evidence_kind": "production_owner_checks_on_live_host_state",
        "sched_ext": format!("{:?}", report.sched_ext),
        "knob_honored": knobs.iter().map(|(name, knob)| ((*name).to_owned(), serde_json::json!(knob_is_honored(ops, *knob)))).collect::<serde_json::Map<_, _>>(),
        "owner_denial": operations.iter().map(|(name, op)| ((*name).to_owned(), serde_json::json!(owner_denial(Some(&report), *op)))).collect::<serde_json::Map<_, _>>(),
        "planner_refusal_condition": "SchedulerSemanticsUnverified is raised when a Nice or Uclamp candidate meets SchedExt::Enabled with knob_is_honored == false",
        "planner_would_refuse": ops.is_some_and(|name| !knob_is_honored(Some(name), Knob::Nice) && !knob_is_honored(Some(name), Knob::Uclamp)),
    });
    serde_json::to_writer_pretty(out, &value)?;
    Ok(())
}
