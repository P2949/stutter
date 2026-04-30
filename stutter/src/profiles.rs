use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use anyhow::Context;
use regex::Regex;
use serde::Deserialize;

use crate::{
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{self, TaskClass, TaskInfo},
};

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub rules: Vec<ProfileRule>,
}

#[derive(Clone, Debug)]
pub struct ProfileRule {
    pub affinity: CpuMask,
    pub match_class: Vec<TaskClass>,
    pub match_comm: Vec<CompiledPattern>,
}

#[derive(Clone, Debug)]
pub struct CompiledPattern {
    raw: String,
    regex: Option<Regex>,
}

#[derive(Default)]
pub struct ProfileApplyCache {
    known_correct: BTreeSet<ProfileApplyCacheKey>,
}

impl ProfileApplyCache {
    pub fn clear(&mut self) {
        self.known_correct.clear();
    }
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
    load_profiles(path)?.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "profile file {} did not contain [[profile]]",
            path.display()
        )
    })
}

pub fn load_profiles(path: &Path) -> anyhow::Result<Vec<Profile>> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile {}", path.display()))?;
    parse_profiles(&data).with_context(|| format!("failed to parse profile {}", path.display()))
}

pub fn apply_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<Vec<AffinityRecord>> {
    apply_profile_to_tree_with_cache(tree_pid, profile, force_restore_overwrite, dry_run, None)
}

pub fn apply_profile_to_tree_cached(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    cache: &mut ProfileApplyCache,
) -> anyhow::Result<Vec<AffinityRecord>> {
    apply_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        Some(cache),
    )
}

fn apply_profile_to_tree_with_cache(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    mut cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<Vec<AffinityRecord>> {
    let snapshot = process_tree::target_snapshot(&[], &[tree_pid]);
    let planned = planned_affinity_changes(&snapshot.tasks, profile, cache.as_deref_mut())?;
    if planned.is_empty() {
        return Ok(Vec::new());
    }

    let restore_path = affinity::default_restore_path();
    let mut applied_records = Vec::new();

    if dry_run {
        for planned in &planned {
            log::info!(
                "dry_run: would apply mask {} to TID {}",
                planned.record.applied_mask.to_range_string(),
                planned.record.tid
            );
            applied_records.push(planned.record.clone());
        }
        applied_records.sort_by_key(|record| record.tid);
        return Ok(applied_records);
    }

    let restore_records = planned
        .iter()
        .map(|planned| planned.record.clone())
        .collect::<Vec<_>>();
    affinity::save_merged_restore_state(&restore_path, &restore_records, force_restore_overwrite)?;

    for planned in &planned {
        match affinity::set_affinity_raw(planned.record.tid, &planned.record.applied_mask) {
            Ok(()) => {
                applied_records.push(planned.record.clone());
                if let Some(cache) = cache.as_deref_mut() {
                    cache.known_correct.insert(planned.cache_key.clone());
                }
            }
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                // ESRCH during set_affinity is fine because restore_all skips dead TIDs.
            }
            Err(err) => {
                anyhow::bail!(
                    "failed to set affinity for TID {}: {err}",
                    planned.record.tid
                );
            }
        }
    }

    applied_records.sort_by_key(|record| record.tid);

    Ok(applied_records)
}

fn planned_affinity_changes(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<Vec<PlannedAffinityChange>> {
    planned_affinity_changes_with_reader(tasks, profile, cache, affinity::read_allowed_mask_raw)
}

fn planned_affinity_changes_with_reader<F>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    mut cache: Option<&mut ProfileApplyCache>,
    mut read_allowed_mask: F,
) -> anyhow::Result<Vec<PlannedAffinityChange>>
where
    F: FnMut(u32) -> io::Result<CpuMask>,
{
    let mut planned = Vec::new();
    let mut seen_cache_keys = BTreeSet::new();

    for task in tasks.values() {
        let mut matched_rule = None;
        for rule in &profile.rules {
            if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
                continue;
            }

            if !rule.match_comm.is_empty() {
                let comms = [&task.comm, task.process_comm.as_ref()];
                let mut comm_match = false;

                for pattern in &rule.match_comm {
                    if comms.iter().any(|comm| pattern.matches(comm)) {
                        comm_match = true;
                        break;
                    }
                }

                if !comm_match {
                    continue;
                }
            }

            matched_rule = Some(rule);
            break;
        }

        let Some(rule) = matched_rule else {
            continue;
        };

        let cache_key = ProfileApplyCacheKey::new(task, &rule.affinity);
        seen_cache_keys.insert(cache_key.clone());

        if cache
            .as_ref()
            .is_some_and(|cache| cache.known_correct.contains(&cache_key))
        {
            continue;
        }

        let original_mask = match read_allowed_mask(task.tid) {
            Ok(mask) => mask,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                continue; // Task is dead, skip it.
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "failed to read CPU affinity for TID {}: {err}",
                    task.tid
                ));
            }
        };
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

impl CompiledPattern {
    fn new(raw: String) -> anyhow::Result<Self> {
        let regex = if raw.len() >= 2 && raw.starts_with('/') && raw.ends_with('/') {
            Some(
                Regex::new(&raw[1..raw.len() - 1])
                    .with_context(|| format!("invalid profile regex '{}'", raw))?,
            )
        } else {
            None
        };

        Ok(Self { raw, regex })
    }

    #[cfg(test)]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    fn matches(&self, value: &str) -> bool {
        if let Some(regex) = &self.regex {
            regex.is_match(value)
        } else {
            value
                .to_ascii_lowercase()
                .contains(&self.raw.to_ascii_lowercase())
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

    let online = affinity::CpuMask::online_cpus()
        .context("failed to read online CPU mask while validating profile")?;

    for (i, rule) in profile.rules.iter().enumerate() {
        if rule.affinity.is_empty() {
            anyhow::bail!("profile rule {} is missing affinity", i);
        }
        if !rule.affinity.is_subset_of(&online) {
            anyhow::bail!(
                "profile rule {} requests CPUs not currently online. Online: {}",
                i,
                online.to_range_string()
            );
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
    rules: Vec<ProfileRuleToml>,
}

#[derive(Deserialize)]
struct ProfileRuleToml {
    affinity: String,
    #[serde(default)]
    match_class: Vec<String>,
    #[serde(default)]
    match_comm: Vec<String>,
}

impl ProfileToml {
    fn try_into_profile(self) -> anyhow::Result<Profile> {
        let mut rules = Vec::new();

        for rule in self.rules {
            let mut match_class = Vec::new();
            for class_name in rule.match_class {
                match_class.push(parse_task_class(&class_name)?);
            }

            let match_comm = rule
                .match_comm
                .into_iter()
                .map(CompiledPattern::new)
                .collect::<anyhow::Result<Vec<_>>>()?;

            rules.push(ProfileRule {
                affinity: CpuMask::parse(&rule.affinity)?,
                match_class,
                match_comm,
            });
        }

        Ok(Profile {
            name: self.name,
            rules,
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

            [[profile.rules]]
            affinity = "0-3"
            match_class = ["Game"]
            match_comm = ["RenderThread", "Main"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "kcd # not a comment");
        let rule = &profiles[0].rules[0];
        assert_eq!(rule.affinity.to_range_string(), "0-3");
        assert_eq!(rule.match_class, vec![TaskClass::Game]);
        assert_eq!(
            rule.match_comm
                .iter()
                .map(CompiledPattern::raw)
                .collect::<Vec<_>>(),
            vec!["RenderThread", "Main"]
        );
    }

    #[test]
    fn match_comm_treats_metacharacters_as_literals_unless_slash_delimited() {
        let literal = CompiledPattern::new("KingdomCome.exe".to_owned()).unwrap();
        assert!(literal.matches("KingdomCome.exe"));
        assert!(literal.matches("kingdomcome.exe"));
        assert!(!literal.matches("KingdomComeXexe"));

        let regex = CompiledPattern::new("/KingdomCome[.]exe$/".to_owned()).unwrap();
        assert!(regex.matches("KingdomCome.exe"));
        assert!(!regex.matches("kingdomcome.exe"));
        assert!(!regex.matches("KingdomComeXexe"));

        let literal_bracket = CompiledPattern::new("[".to_owned()).unwrap();
        assert!(literal_bracket.matches("renderer[0]"));
        assert!(CompiledPattern::new("/[/".to_owned()).is_err());
    }

    #[test]
    fn profile_apply_cache_skips_unchanged_known_correct_tasks() {
        let task = TaskInfo {
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "RenderThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(70),
            task_starttime_ticks: Some(70),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
        };
        let tasks = BTreeMap::from([(7, task)]);
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-1").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
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

        cache.clear();
        let third =
            planned_affinity_changes_with_reader(&tasks, &profile, Some(&mut cache), |_| {
                reads += 1;
                Ok(CpuMask::parse("0-1").unwrap())
            })
            .unwrap();
        assert!(third.is_empty());
        assert_eq!(reads, 2);
    }
}
