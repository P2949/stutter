use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Context;
use serde::Deserialize;

use crate::{
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{self, TaskClass, TaskInfo},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    pub name: String,
    pub classes: BTreeMap<TaskClass, ProfileClassRule>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileClassRule {
    pub affinity: CpuMask,
    pub match_comm: Vec<String>,
}

pub fn load_first_profile(path: &Path) -> anyhow::Result<Profile> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile {}", path.display()))?;
    parse_profiles(&data)?.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "profile file {} did not contain [[profile]]",
            path.display()
        )
    })
}

pub fn apply_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
) -> anyhow::Result<Vec<AffinityRecord>> {
    let snapshot = process_tree::target_snapshot(&[], &[tree_pid]);
    let mut records = planned_affinity_records(&snapshot.tasks, profile)?;

    records.sort_by_key(|record| record.tid);
    if records.is_empty() {
        return Ok(records);
    }

    let restore_path = affinity::default_restore_path();
    affinity::save_merged_restore_state(&restore_path, &records, force_restore_overwrite)?;

    for record in &records {
        affinity::set_affinity(record.tid, record.applied_mask)?;
    }

    Ok(records)
}

fn planned_affinity_records(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
) -> anyhow::Result<Vec<AffinityRecord>> {
    let mut records = Vec::new();

    for task in tasks.values() {
        let Some(rule) = profile.classes.get(&task.class) else {
            continue;
        };
        if !rule.match_comm.is_empty()
            && !rule
                .match_comm
                .iter()
                .any(|pattern| task.comm.contains(pattern) || task.process_comm.contains(pattern))
        {
            continue;
        }

        let original_mask = affinity::read_allowed_mask(task.tid)?;
        if original_mask == rule.affinity {
            continue;
        }

        records.push(AffinityRecord {
            tid: task.tid,
            original_mask,
            applied_mask: rule.affinity,
        });
    }

    Ok(records)
}

fn parse_profiles(data: &str) -> anyhow::Result<Vec<Profile>> {
    let file = toml::from_str::<ProfilesFile>(data)?;
    file.profile
        .into_iter()
        .map(ProfileToml::try_into_profile)
        .map(|profile| profile.and_then(validate_profile))
        .collect()
}

fn validate_profile(profile: Profile) -> anyhow::Result<Profile> {
    if profile.name.is_empty() {
        anyhow::bail!("profile name must not be empty");
    }
    for (class, rule) in &profile.classes {
        if rule.affinity.0 == 0 {
            anyhow::bail!("profile class {class} is missing affinity");
        }
    }
    Ok(profile)
}

#[derive(Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profile: Vec<ProfileToml>,
}

#[derive(Deserialize)]
struct ProfileToml {
    name: String,
    #[serde(default)]
    classes: BTreeMap<String, ProfileClassToml>,
}

#[derive(Deserialize)]
struct ProfileClassToml {
    affinity: String,
    #[serde(default)]
    match_comm: Vec<String>,
}

impl ProfileToml {
    fn try_into_profile(self) -> anyhow::Result<Profile> {
        let mut classes = BTreeMap::new();

        for (class_name, rule) in self.classes {
            classes.insert(
                parse_task_class(&class_name)?,
                ProfileClassRule {
                    affinity: CpuMask::parse(&rule.affinity)?,
                    match_comm: rule.match_comm,
                },
            );
        }

        Ok(Profile {
            name: self.name,
            classes,
        })
    }
}

fn parse_task_class(value: &str) -> anyhow::Result<TaskClass> {
    match value {
        "Game" => Ok(TaskClass::Game),
        "GameHelper" => Ok(TaskClass::GameHelper),
        "Launcher" => Ok(TaskClass::Launcher),
        "WineServer" => Ok(TaskClass::WineServer),
        "GameScope" => Ok(TaskClass::GameScope),
        "Compositor" => Ok(TaskClass::Compositor),
        "SteamRuntime" => Ok(TaskClass::SteamRuntime),
        "Helper" => Ok(TaskClass::Helper),
        "Unknown" => Ok(TaskClass::Unknown),
        _ => anyhow::bail!("unknown task class {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_profile() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "kcd # not a comment"

            [profile.classes.Game]
            affinity = "0-3"
            match_comm = ["RenderThread", "Main"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "kcd # not a comment");
        let rule = profiles[0].classes.get(&TaskClass::Game).unwrap();
        assert_eq!(rule.affinity, CpuMask(0b1111));
        assert_eq!(rule.match_comm, vec!["RenderThread", "Main"]);
    }
}
