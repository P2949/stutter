#![allow(dead_code)]

pub mod cpu_affinity;
pub mod runner;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SafetyClass {
    ObserveOnly,
    ReversibleLowRisk,
    ReversibleMediumRisk,
    HighRisk,
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
    pub restore_path: Option<std::path::PathBuf>,
    pub affected_tasks: usize,
}

pub trait TuningAction {
    fn id(&self) -> ActionId;
    fn describe(&self) -> String;
    fn safety_class(&self) -> SafetyClass;
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>>;
    fn dry_run(&self) -> anyhow::Result<ActionState>;
    fn apply(&self) -> anyhow::Result<RollbackToken>;
    fn verify(&self) -> anyhow::Result<ActionState>;
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()>;
}
