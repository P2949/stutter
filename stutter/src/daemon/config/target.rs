use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTargetConfig {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub watch_process: Option<String>,
    pub require_explicit_target: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonCgroupTargetsConfig {
    pub interactive_cgroup: Option<PathBuf>,
    pub background_cgroup: Option<PathBuf>,
    pub game_cgroup: Option<PathBuf>,
    pub compile_cgroup: Option<PathBuf>,
}
impl DaemonCgroupTargetsConfig {
    pub fn is_empty(&self) -> bool {
        self.interactive_cgroup.is_none()
            && self.background_cgroup.is_none()
            && self.game_cgroup.is_none()
            && self.compile_cgroup.is_none()
    }

    pub fn target_for_role(&self, role: CgroupTargetRole) -> Option<&Path> {
        match role {
            CgroupTargetRole::Interactive => self.interactive_cgroup.as_deref(),
            CgroupTargetRole::Background => self.background_cgroup.as_deref(),
            CgroupTargetRole::Game => self.game_cgroup.as_deref(),
            CgroupTargetRole::Compile => self.compile_cgroup.as_deref(),
        }
    }

    pub fn contains_path(&self, path: &Path) -> bool {
        let Ok(candidate) = normalize_cgroup_target_path(path) else {
            return false;
        };

        self.named_targets().into_iter().any(|(_, target)| {
            normalize_cgroup_target_path(target).is_ok_and(|known| known == candidate)
        })
    }

    pub fn named_targets(&self) -> Vec<(&'static str, &Path)> {
        [
            (
                CgroupTargetRole::Interactive.as_str(),
                self.interactive_cgroup.as_deref(),
            ),
            (
                CgroupTargetRole::Background.as_str(),
                self.background_cgroup.as_deref(),
            ),
            (CgroupTargetRole::Game.as_str(), self.game_cgroup.as_deref()),
            (
                CgroupTargetRole::Compile.as_str(),
                self.compile_cgroup.as_deref(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, target)| target.map(|target| (name, target)))
        .collect()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        for (name, target) in self.named_targets() {
            normalize_cgroup_target_path(target)
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("invalid {name}_cgroup target: {err}"))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CgroupTargetRole {
    Interactive,
    Background,
    Game,
    Compile,
}

impl CgroupTargetRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Background => "background",
            Self::Game => "game",
            Self::Compile => "compile",
        }
    }
}

pub fn normalize_cgroup_target_path(path: &Path) -> anyhow::Result<String> {
    if !path.is_absolute() {
        anyhow::bail!("cgroup target path must be absolute within the cgroup v2 namespace");
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!(
                    "cgroup target path must not contain parent traversal: {}",
                    path.display()
                )
            }
            Component::Prefix(_) => {
                anyhow::bail!(
                    "cgroup target path must not contain platform prefixes: {}",
                    path.display()
                )
            }
        }
    }

    if parts.is_empty() {
        anyhow::bail!("cgroup target path must not be the cgroup root");
    }

    Ok(format!("/{}", parts.join("/")))
}
