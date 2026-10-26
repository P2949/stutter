//! Shared fixtures for the campaign tests.
use std::path::{Path, PathBuf};

use super::{
    graph::Graph,
    manifest::CampaignManifest,
    model::CampaignState,
    state::{CampaignDir, InitInputs},
};

mod backend;
mod bootstrap;
mod capture_exec;
mod corpus;
mod evidence;
mod frame_score;
mod graph;
mod init;
mod integration;
mod invalidation;
mod kernels;
mod landing;
mod manifest;
mod overhead_real;
mod policy;
mod policy_tasks;
mod release;
mod release_lifecycle;
mod scenarios;
mod state;
mod workloads;

pub(super) const MANIFEST_TEXT: &str =
    include_str!("../../../../docs/validation/core-validation.toml");

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A unique scratch directory removed on drop.
pub(super) struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "stutter-campaign-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) const BASE: &str = "0123456789abcdef0123456789abcdef01234567";

pub(super) fn manifest() -> CampaignManifest {
    CampaignManifest::parse(MANIFEST_TEXT).unwrap()
}

pub(super) fn fresh(scratch: &Scratch) -> (CampaignDir, CampaignState, Graph) {
    let graph = Graph::core().unwrap();
    let dir = CampaignDir::under(scratch.path(), "test-campaign");
    let state = CampaignState::init(
        InitInputs {
            campaign_id: "test-campaign".to_owned(),
            repository: scratch.path().to_path_buf(),
            evidence_base: BASE.to_owned(),
            manifest_sha256: "a".repeat(64),
            cargo_lock_sha256: "c".repeat(64),
            xtask_sha256: "b".repeat(64),
            stutter_sha256: "e".repeat(64),
            uid: 1000,
            gid: 1000,
        },
        &graph,
        &manifest(),
    );
    (dir, state, graph)
}

pub(super) fn scenarios_set() -> super::scenarios::model::ScenarioSet {
    super::scenarios::model::ScenarioSet::parse(include_str!(
        "../../../../docs/validation/scenarios/core-validation.toml"
    ))
    .unwrap()
}
