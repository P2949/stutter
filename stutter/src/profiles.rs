use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

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

#[derive(Default)]
pub struct ProfileApplyCache {
    known_correct: BTreeSet<ProfileApplyCacheKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProfileApplyCacheKey {
    tid: u32,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    desired_mask: CpuMask,
}

struct PlannedAffinityChange {
    record: AffinityRecord,
    cache_key: ProfileApplyCacheKey,
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
    apply_profile_to_tree_with_cache(tree_pid, profile, force_restore_overwrite, None)
}

pub fn apply_profile_to_tree_cached(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    cache: &mut ProfileApplyCache,
) -> anyhow::Result<Vec<AffinityRecord>> {
    apply_profile_to_tree_with_cache(tree_pid, profile, force_restore_overwrite, Some(cache))
}

fn apply_profile_to_tree_with_cache(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    mut cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<Vec<AffinityRecord>> {
    let snapshot = process_tree::target_snapshot(&[], &[tree_pid]);
    let planned = planned_affinity_changes(&snapshot.tasks, profile, cache.as_deref_mut())?;
    let mut records = planned
        .iter()
        .map(|planned| planned.record.clone())
        .collect::<Vec<_>>();

    records.sort_by_key(|record| record.tid);
    if records.is_empty() {
        return Ok(records);
    }

    let restore_path = affinity::default_restore_path();
    affinity::save_merged_restore_state(&restore_path, &records, force_restore_overwrite)?;

    for planned in &planned {
        affinity::set_affinity(planned.record.tid, planned.record.applied_mask.clone())?;
        if let Some(cache) = cache.as_deref_mut() {
            cache.known_correct.insert(planned.cache_key.clone());
        }
    }

    Ok(records)
}

fn planned_affinity_changes(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<Vec<PlannedAffinityChange>> {
    planned_affinity_changes_with_reader(tasks, profile, cache, affinity::read_allowed_mask)
}

fn planned_affinity_changes_with_reader<F>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    mut cache: Option<&mut ProfileApplyCache>,
    mut read_allowed_mask: F,
) -> anyhow::Result<Vec<PlannedAffinityChange>>
where
    F: FnMut(u32) -> anyhow::Result<CpuMask>,
{
    let mut planned = Vec::new();
    let mut seen_cache_keys = BTreeSet::new();

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

        let cache_key = ProfileApplyCacheKey::new(task, &rule.affinity);
        seen_cache_keys.insert(cache_key.clone());

        if cache
            .as_ref()
            .is_some_and(|cache| cache.known_correct.contains(&cache_key))
        {
            continue;
        }

        let original_mask = read_allowed_mask(task.tid)?;
        if original_mask == rule.affinity {
            if let Some(cache) = cache.as_mut() {
                cache.known_correct.insert(cache_key);
            }
            continue;
        }

        planned.push(PlannedAffinityChange {
            record: AffinityRecord {
                tid: task.tid,
                original_mask,
                applied_mask: rule.affinity.clone(),
            },
            cache_key,
        });
    }

    if let Some(cache) = cache.as_mut() {
        cache
            .known_correct
            .retain(|cache_key| seen_cache_keys.contains(cache_key));
    }

    Ok(planned)
}

impl ProfileApplyCacheKey {
    fn new(task: &TaskInfo, desired_mask: &CpuMask) -> Self {
        Self {
            tid: task.tid,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            desired_mask: desired_mask.clone(),
        }
    }
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
        if rule.affinity.is_empty() {
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
        assert_eq!(rule.affinity.to_range_string(), "0-3");
        assert_eq!(rule.match_comm, vec!["RenderThread", "Main"]);
    }

    #[test]
    fn profile_apply_cache_skips_unchanged_known_correct_tasks() {
        let task = TaskInfo {
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "RenderThread".to_owned(),
            process_comm: "game".to_owned(),
            process_starttime_ticks: Some(70),
            task_starttime_ticks: Some(70),
            class: TaskClass::Game,
        };
        let tasks = BTreeMap::from([(7, task)]);
        let profile = Profile {
            name: "test".to_owned(),
            classes: BTreeMap::from([(
                TaskClass::Game,
                ProfileClassRule {
                    affinity: CpuMask::parse("0-1").unwrap(),
                    match_comm: Vec::new(),
                },
            )]),
        };
        let mut cache = ProfileApplyCache::default();
        let mut reads = 0;

        let first =
            planned_affinity_changes_with_reader(&tasks, &profile, Some(&mut cache), |tid| {
                reads += 1;
                assert_eq!(tid, 7);
                Ok(CpuMask::parse("0-1").unwrap())
            })
            .unwrap();
        assert!(first.is_empty());
        assert_eq!(reads, 1);

        let second =
            planned_affinity_changes_with_reader(&tasks, &profile, Some(&mut cache), |_| {
                reads += 1;
                Ok(CpuMask::parse("0-1").unwrap())
            })
            .unwrap();
        assert!(second.is_empty());
        assert_eq!(reads, 1);
    }
}
