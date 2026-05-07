#![allow(dead_code)]

pub mod cpu_affinity;
pub mod runner;

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SafetyClass {
    ObserveOnly,
    ReversibleLowRisk,
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionScope {
    Task { tid: u32 },
    ProcessTree { root_pid: u32 },
    Cgroup { path: PathBuf },
    Device { id: String },
    SystemWide,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionWarning {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionState {
    pub applied: bool,
    pub affected_tasks: usize,
    pub warnings: Vec<ActionWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackToken {
    pub kind: String,
    pub restore_path: Option<PathBuf>,
    pub affected_tasks: usize,
}

pub trait TuningAction {
    fn id(&self) -> ActionId;
    fn describe(&self) -> String;
    fn action_kind(&self) -> &'static str;
    fn scope(&self) -> ActionScope;
    fn cooldown_hint(&self) -> Duration;
    fn requires_privilege(&self) -> bool;
    fn reversible(&self) -> bool;
    fn safety_class(&self) -> SafetyClass;
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>>;
    fn dry_run(&self) -> anyhow::Result<ActionState>;
    fn apply(&self) -> anyhow::Result<RollbackToken>;
    fn verify(&self) -> anyhow::Result<ActionState>;
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()>;
}
