use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Context;

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
    profile_path: &Path,
) -> anyhow::Result<Vec<AffinityRecord>> {
    let profile = load_first_profile(profile_path)?;
    let snapshot = process_tree::target_snapshot(&[], &[tree_pid]);
    let mut records = planned_affinity_records(&snapshot.tasks, &profile)?;

    records.sort_by_key(|record| record.tid);
    if records.is_empty() {
        return Ok(records);
    }

    let restore_path = affinity::default_restore_path();
    affinity::save_restore_state(&restore_path, &records)?;

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
    let mut profiles = Vec::new();
    let mut current_profile: Option<Profile> = None;
    let mut current_class: Option<TaskClass> = None;

    for raw_line in data.lines() {
        let line = raw_line
            .split_once('#')
            .map_or(raw_line, |(line, _)| line)
            .trim();
        if line.is_empty() {
            continue;
        }

        if line == "[[profile]]" {
            if let Some(profile) = current_profile.take() {
                profiles.push(validate_profile(profile)?);
            }
            current_profile = Some(Profile {
                name: String::new(),
                classes: BTreeMap::new(),
            });
            current_class = None;
            continue;
        }

        if let Some(class_name) = line
            .strip_prefix("[profile.classes.")
            .and_then(|line| line.strip_suffix(']'))
        {
            current_class = Some(parse_task_class(class_name)?);
            let profile = current_profile
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("class table appeared before [[profile]]"))?;
            profile
                .classes
                .entry(current_class.unwrap())
                .or_insert(ProfileClassRule {
                    affinity: CpuMask(0),
                    match_comm: Vec::new(),
                });
            continue;
        }

        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid profile line: {line}"))?;
        let key = key.trim();
        let value = value.trim();

        let profile = current_profile
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("key {key} appeared before [[profile]]"))?;

        match (current_class, key) {
            (None, "name") => profile.name = parse_string(value)?,
            (Some(class), "affinity") => {
                profile
                    .classes
                    .get_mut(&class)
                    .expect("class rule should exist")
                    .affinity = CpuMask::parse(&parse_string(value)?)?;
            }
            (Some(class), "match_comm") => {
                profile
                    .classes
                    .get_mut(&class)
                    .expect("class rule should exist")
                    .match_comm = parse_string_array(value)?;
            }
            _ => anyhow::bail!("unsupported profile key {key}"),
        }
    }

    if let Some(profile) = current_profile {
        profiles.push(validate_profile(profile)?);
    }

    Ok(profiles)
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

fn parse_string(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        anyhow::bail!("expected quoted string, got {value}");
    };
    Ok(value.to_owned())
}

fn parse_string_array(value: &str) -> anyhow::Result<Vec<String>> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        anyhow::bail!("expected string array, got {value}");
    };

    inner
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_profile() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "kcd"

            [profile.classes.Game]
            affinity = "0-3"
            match_comm = ["RenderThread", "Main"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "kcd");
        let rule = profiles[0].classes.get(&TaskClass::Game).unwrap();
        assert_eq!(rule.affinity, CpuMask(0b1111));
        assert_eq!(rule.match_comm, vec!["RenderThread", "Main"]);
    }
}
