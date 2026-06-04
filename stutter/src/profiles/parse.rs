//! Profile TOML loading, parsing, and validation.
//!
//! Owns filesystem loading, TOML DTOs, symbolic value parsing, and profile validation. Does not own
//! action planning/application, matching, warning presentation, or rendering generated TOML.

use std::{fs, path::Path};

use anyhow::Context;
use serde::Deserialize;

use super::{Profile, ProfileRule, warnings::profile_rule_overlap_warnings};
use crate::{
    actions::ioprio::IoPrioValue,
    affinity::{self, CpuMask},
    error::ProfileError,
    process_tree::{CompiledPattern, TaskClass},
};

pub fn load_selected_profile(path: &Path, profile_name: Option<&str>) -> anyhow::Result<Profile> {
    let Some(name) = profile_name else {
        return load_first_profile(path);
    };

    let profiles = load_profiles(path)?;

    let available = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    profiles
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "profile '{}' not found in {}; available profiles: {}",
                name,
                path.display(),
                if available.is_empty() {
                    "<none>"
                } else {
                    available.as_str()
                }
            )
        })
}

fn load_first_profile(path: &Path) -> anyhow::Result<Profile> {
    load_profiles(path)?.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "profile file {} did not contain [[profile]]",
            path.display()
        )
    })
}

pub fn load_profiles(path: &Path) -> Result<Vec<Profile>, ProfileError> {
    let data = fs::read_to_string(path).map_err(|source| ProfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_profiles(&data).map_err(|source| ProfileError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn parse_profiles(data: &str) -> anyhow::Result<Vec<Profile>> {
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
        if rule.affinity.is_none() && rule.nice.is_none() && rule.ionice.is_none() {
            anyhow::bail!(
                "profile rule {} must specify at least one action field: affinity, nice, or ionice",
                i
            );
        }
        if let Some(affinity) = &rule.affinity {
            if affinity.is_empty() {
                anyhow::bail!("profile rule {} has empty affinity", i);
            }
            if !affinity.is_subset_of(&online) {
                anyhow::bail!(
                    "profile rule {} requests CPUs not currently online. Online: {}",
                    i,
                    online.to_range_string()
                );
            }
        }
    }

    for warning in profile_rule_overlap_warnings(&profile.rules) {
        log::warn!(
            "profile_rule_overlap profile={} earlier_rule={} later_rule={} message=\"rules are first-match-wins; later rule may be shadowed\"",
            profile.name,
            warning.earlier_rule,
            warning.later_rule
        );
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
    affinity: Option<String>,
    nice: Option<i32>,
    ionice: Option<String>,
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

            let affinity = rule
                .affinity
                .as_deref()
                .map(parse_affinity_value)
                .transpose()?;
            let ionice = rule.ionice.as_deref().map(parse_ionice_value).transpose()?;

            if rule.nice.is_none() && affinity.is_none() && ionice.is_none() {
                anyhow::bail!(
                    "profile rule must specify at least one action field: affinity, nice, or ionice"
                );
            }

            if let Some(nice) = rule.nice
                && !(-20..=19).contains(&nice)
            {
                anyhow::bail!("nice value {nice} is outside Linux range -20..=19");
            }

            rules.push(ProfileRule {
                affinity,
                nice: rule.nice,
                ionice,
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

fn parse_affinity_value(value: &str) -> anyhow::Result<CpuMask> {
    if value.trim() == "online" {
        CpuMask::online_cpus()
    } else {
        CpuMask::parse(value)
    }
}

fn parse_ionice_value(value: &str) -> anyhow::Result<IoPrioValue> {
    let trimmed = value.trim().to_ascii_lowercase();

    match trimmed.as_str() {
        "idle" => return Ok(IoPrioValue::idle()),
        "none" => return Ok(IoPrioValue::none()),
        "best-effort" | "be" | "realtime" | "rt" => {
            anyhow::bail!("ionice value {value:?} requires a level 0..=7")
        }
        _ => {}
    }

    let Some((class, level)) = trimmed.split_once(':') else {
        anyhow::bail!("invalid ionice value {value:?}");
    };
    let level = level
        .parse::<u8>()
        .with_context(|| format!("invalid ionice level in {value:?}"))?;

    let parsed = match class {
        "best-effort" | "be" => IoPrioValue::best_effort(level),
        "realtime" | "rt" => IoPrioValue::realtime(level),
        "idle" => anyhow::bail!("ionice class idle must not specify a level"),
        "none" => anyhow::bail!("ionice class none must not specify a level"),
        _ => anyhow::bail!("invalid ionice class {class:?}"),
    };

    parsed.encode()?;
    Ok(parsed)
}

fn parse_task_class(value: &str) -> anyhow::Result<TaskClass> {
    match value {
        "Game" => Ok(TaskClass::Game),
        "GameRenderThread" => Ok(TaskClass::GameRenderThread),
        "GameWorkerThread" => Ok(TaskClass::GameWorkerThread),
        "GameHelper" => Ok(TaskClass::GameHelper),
        "Launcher" => Ok(TaskClass::Launcher),
        "WineServer" => Ok(TaskClass::WineServer),
        "GameScope" => Ok(TaskClass::GameScope),
        "Compositor" => Ok(TaskClass::Compositor),
        "AudioRealtime" => Ok(TaskClass::AudioRealtime),
        "Input" => Ok(TaskClass::Input),
        "BrowserForeground" => Ok(TaskClass::BrowserForeground),
        "BrowserBackground" => Ok(TaskClass::BrowserBackground),
        "BrowserRenderer" => Ok(TaskClass::BrowserRenderer),
        "BrowserGpu" => Ok(TaskClass::BrowserGpu),
        "BrowserNetwork" => Ok(TaskClass::BrowserNetwork),
        "Compiler" => Ok(TaskClass::Compiler),
        "Linker" => Ok(TaskClass::Linker),
        "Indexer" => Ok(TaskClass::Indexer),
        "PackageManager" => Ok(TaskClass::PackageManager),
        "BuildJob" => Ok(TaskClass::BuildJob),
        "StorageDaemon" => Ok(TaskClass::StorageDaemon),
        "NetworkDaemon" => Ok(TaskClass::NetworkDaemon),
        "KernelThread" => Ok(TaskClass::KernelThread),
        "IrqThread" => Ok(TaskClass::IrqThread),
        "Editor" => Ok(TaskClass::Editor),
        "Terminal" => Ok(TaskClass::Terminal),
        "Shell" => Ok(TaskClass::Shell),
        "Media" => Ok(TaskClass::Media),
        "Recorder" => Ok(TaskClass::Recorder),
        "VirtualMachine" => Ok(TaskClass::VirtualMachine),
        "SteamRuntime" => Ok(TaskClass::SteamRuntime),
        "Render" => Ok(TaskClass::Render),
        "Helper" => Ok(TaskClass::Helper),
        "Service" => Ok(TaskClass::Service),
        "Unknown" => Ok(TaskClass::Unknown),
        _ => anyhow::bail!("unknown task class {value}"),
    }
}
