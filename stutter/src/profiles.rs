use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use anyhow::Context;
use serde::Deserialize;

use crate::{
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{self, CompiledPattern, TaskClass, TaskInfo},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCpuWarning {
    pub rule_index: usize,
    pub requested: String,
    pub online: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRuleOverlapWarning {
    pub earlier_rule: usize,
    pub later_rule: usize,
}

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileApplySummary {
    pub checked_tasks: usize,
    pub pending_changes: usize,
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
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );

    if let Err(err) = warn_profile_offline_cpus(profile) {
        log::warn!("profile_online_cpu_check_failed err={err:#}");
    }

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

        // Ensure we only create restore records with full identity. If the
        // process or task starttime ticks are missing then we must not record
        // a partial identity that could later be mis-applied or left in an
        // inconsistent state. Skip and warn instead.
        if task.process_starttime_ticks.is_none() || task.task_starttime_ticks.is_none() {
            log::warn!(
                "profile_skip_incomplete_identity tid={} comm={} process_pid={}",
                task.tid,
                task.comm,
                task.process_pid
            );
            continue;
        }

        planned.push(PlannedAffinityChange {
            record: AffinityRecord {
                tid: task.tid,
                process_pid: Some(task.process_pid),
                process_starttime_ticks: task.process_starttime_ticks,
                task_starttime_ticks: task.task_starttime_ticks,
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

pub fn profile_apply_summary_for_tree(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<ProfileApplySummary> {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );

    profile_apply_summary(&snapshot.tasks, profile)
}

fn profile_apply_summary(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
) -> anyhow::Result<ProfileApplySummary> {
    profile_apply_summary_with_reader(tasks, profile, affinity::read_allowed_mask_raw)
}

fn profile_apply_summary_with_reader<F>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    mut read_allowed_mask: F,
) -> anyhow::Result<ProfileApplySummary>
where
    F: FnMut(u32) -> io::Result<CpuMask>,
{
    let mut summary = ProfileApplySummary::default();

    for task in tasks.values() {
        let Some(rule) = matching_profile_rule(task, profile) else {
            continue;
        };

        let original_mask = match read_allowed_mask(task.tid) {
            Ok(mask) => mask,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                continue;
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "failed to read CPU affinity for TID {}: {err}",
                    task.tid
                ));
            }
        };

        summary.checked_tasks += 1;

        if original_mask != rule.affinity {
            summary.pending_changes += 1;
        }
    }

    Ok(summary)
}

fn matching_profile_rule<'a>(task: &TaskInfo, profile: &'a Profile) -> Option<&'a ProfileRule> {
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

        return Some(rule);
    }

    None
}

pub fn profile_matched_task_count_for_tree(tree_pid: u32, profile: &Profile) -> usize {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    profile_matched_task_count(&snapshot.tasks, profile)
}

pub fn profile_matched_task_count(tasks: &BTreeMap<u32, TaskInfo>, profile: &Profile) -> usize {
    tasks
        .values()
        .filter(|task| profile_matches_task(task, profile))
        .count()
}

pub fn profile_matches_task(task: &TaskInfo, profile: &Profile) -> bool {
    profile
        .rules
        .iter()
        .any(|rule| profile_rule_matches_task(task, rule))
}

pub fn profile_rule_matches_task(task: &TaskInfo, rule: &ProfileRule) -> bool {
    if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
        return false;
    }

    if !rule.match_comm.is_empty() {
        let comms = [&task.comm, task.process_comm.as_ref()];
        return rule
            .match_comm
            .iter()
            .any(|pattern| comms.iter().any(|comm| pattern.matches(comm)));
    }

    true
}

fn class_dimension_may_overlap(a: &[TaskClass], b: &[TaskClass]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|left| b.contains(left))
}

fn comm_dimension_may_overlap(a: &[CompiledPattern], b: &[CompiledPattern]) -> bool {
    a.is_empty()
        || b.is_empty()
        || a.iter()
            .any(|left| b.iter().any(|right| left.raw() == right.raw()))
}

fn rule_may_overlap(earlier: &ProfileRule, later: &ProfileRule) -> bool {
    let class_overlap = class_dimension_may_overlap(&earlier.match_class, &later.match_class);
    let comm_overlap = comm_dimension_may_overlap(&earlier.match_comm, &later.match_comm);

    class_overlap && comm_overlap
}

pub fn profile_rule_overlap_warnings(rules: &[ProfileRule]) -> Vec<ProfileRuleOverlapWarning> {
    let mut warnings = Vec::new();

    for earlier_rule in 0..rules.len() {
        for later_rule in (earlier_rule + 1)..rules.len() {
            if rule_may_overlap(&rules[earlier_rule], &rules[later_rule]) {
                warnings.push(ProfileRuleOverlapWarning {
                    earlier_rule,
                    later_rule,
                });
            }
        }
    }

    warnings
}

pub fn profile_offline_cpu_warnings(profile: &Profile, online: &CpuMask) -> Vec<ProfileCpuWarning> {
    profile
        .rules
        .iter()
        .enumerate()
        .filter_map(|(idx, rule)| {
            if rule.affinity.is_subset_of(online) {
                None
            } else {
                Some(ProfileCpuWarning {
                    rule_index: idx,
                    requested: rule.affinity.to_range_string(),
                    online: online.to_range_string(),
                })
            }
        })
        .collect()
}

fn warn_profile_offline_cpus(profile: &Profile) -> anyhow::Result<()> {
    let online =
        CpuMask::online_cpus().context("failed to read online CPU mask before applying profile")?;

    for warning in profile_offline_cpu_warnings(profile, &online) {
        log::warn!(
            "profile_rule_offline_cpus rule={} requested={} online={}",
            warning.rule_index,
            warning.requested,
            warning.online
        );
    }

    Ok(())
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
                affinity: parse_affinity_value(&rule.affinity)?,
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
        "Helper" => Ok(TaskClass::Helper),
        "Service" => Ok(TaskClass::Service),
        "Unknown" => Ok(TaskClass::Unknown),
        _ => anyhow::bail!("unknown task class {value}"),
    }
}

pub fn render_profiles_toml(profiles: &[Profile]) -> String {
    let mut out = String::new();

    for profile in profiles {
        out.push_str("[[profile]]\n");
        out.push_str("name = ");
        out.push_str(&toml_quoted_string(&profile.name));
        out.push_str("\n\n");

        for rule in &profile.rules {
            out.push_str("[[profile.rules]]\n");
            out.push_str("affinity = ");
            out.push_str(&toml_quoted_string(&rule.affinity.to_range_string()));
            out.push('\n');

            if !rule.match_class.is_empty() {
                out.push_str("match_class = [");
                for (idx, class) in rule.match_class.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(task_class_toml_name(*class)));
                }
                out.push_str("]\n");
            }

            if !rule.match_comm.is_empty() {
                out.push_str("match_comm = [");
                for (idx, pattern) in rule.match_comm.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(pattern.raw()));
                }
                out.push_str("]\n");
            }

            out.push('\n');
        }
    }

    out
}

fn toml_quoted_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');

    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }

    quoted.push('"');
    quoted
}

fn task_class_toml_name(class: TaskClass) -> &'static str {
    match class {
        TaskClass::Game => "Game",
        TaskClass::GameRenderThread => "GameRenderThread",
        TaskClass::GameWorkerThread => "GameWorkerThread",
        TaskClass::GameHelper => "GameHelper",
        TaskClass::Launcher => "Launcher",
        TaskClass::WineServer => "WineServer",
        TaskClass::GameScope => "GameScope",
        TaskClass::Compositor => "Compositor",
        TaskClass::AudioRealtime => "AudioRealtime",
        TaskClass::Input => "Input",
        TaskClass::BrowserForeground => "BrowserForeground",
        TaskClass::BrowserBackground => "BrowserBackground",
        TaskClass::BrowserRenderer => "BrowserRenderer",
        TaskClass::BrowserGpu => "BrowserGpu",
        TaskClass::BrowserNetwork => "BrowserNetwork",
        TaskClass::Compiler => "Compiler",
        TaskClass::Linker => "Linker",
        TaskClass::Indexer => "Indexer",
        TaskClass::PackageManager => "PackageManager",
        TaskClass::BuildJob => "BuildJob",
        TaskClass::StorageDaemon => "StorageDaemon",
        TaskClass::NetworkDaemon => "NetworkDaemon",
        TaskClass::KernelThread => "KernelThread",
        TaskClass::IrqThread => "IrqThread",
        TaskClass::Editor => "Editor",
        TaskClass::Terminal => "Terminal",
        TaskClass::Shell => "Shell",
        TaskClass::Media => "Media",
        TaskClass::Recorder => "Recorder",
        TaskClass::VirtualMachine => "VirtualMachine",
        TaskClass::SteamRuntime => "SteamRuntime",
        TaskClass::Render => "Render",
        TaskClass::Helper => "Helper",
        TaskClass::Service => "Service",
        TaskClass::Unknown => "Unknown",
    }
}

pub fn generate_topology_template() -> String {
    let mut out = String::new();
    out.push_str("[[profile]]\n");
    out.push_str("name = \"baseline-online\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"online\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\", \"GameScope\", \"Compositor\", \"AudioRealtime\", \"Input\", \"BrowserForeground\", \"BrowserBackground\", \"BrowserRenderer\", \"BrowserGpu\", \"BrowserNetwork\", \"Compiler\", \"Linker\", \"Indexer\", \"PackageManager\", \"BuildJob\", \"StorageDaemon\", \"NetworkDaemon\", \"KernelThread\", \"IrqThread\", \"Editor\", \"Terminal\", \"Shell\", \"Media\", \"Recorder\", \"VirtualMachine\", \"SteamRuntime\", \"Helper\", \"Service\", \"Unknown\"]\n\n");
    out.push_str("[[profile]]\n");
    out.push_str("name = \"game-main-suggested\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"GameScope\", \"Compositor\"]\n");
    out
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
    fn render_profiles_toml_outputs_profile_rules() {
        let profile = Profile {
            name: "generated \"profile\"".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-1").unwrap(),
                match_class: vec![TaskClass::Game, TaskClass::GameRenderThread],
                match_comm: vec![
                    CompiledPattern::new("RenderThread".to_owned()).unwrap(),
                    CompiledPattern::new("Main".to_owned()).unwrap(),
                ],
            }],
        };

        let toml = render_profiles_toml(&[profile]);

        assert!(toml.contains("[[profile]]"));
        assert!(toml.contains("name = \"generated \\\"profile\\\"\""));
        assert!(toml.contains("[[profile.rules]]"));
        assert!(toml.contains("affinity = \"0-1\""));
        assert!(toml.contains("match_class = [\"Game\", \"GameRenderThread\"]"));
        assert!(toml.contains("match_comm = [\"RenderThread\", \"Main\"]"));
    }

    #[test]
    fn profile_parser_accepts_online_affinity() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "baseline-online"

            [[profile.rules]]
            affinity = "online"
            match_class = ["Game"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert!(!profiles[0].rules[0].affinity.is_empty());
    }

    #[test]
    fn invalid_symbolic_affinity_fails_clearly() {
        let err = parse_profiles(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            affinity = "all"
            match_class = ["Game"]
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("invalid CPU id"));
    }

    #[test]
    fn examples_profile_file_parses() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest_dir
            .parent()
            .unwrap()
            .join("examples/profiles/common-game-layouts.toml");
        let profiles = load_profiles(&path).unwrap();

        assert!(!profiles.is_empty());
        assert!(
            profiles
                .iter()
                .any(|profile| profile.name == "baseline-online")
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
    fn profile_match_class_sees_community_rule_game_class() {
        let class = process_tree::classify_task_with_context(
            "KingdomCome",
            "KingdomCome",
            "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
            "/usr/bin/wine",
            "/user.slice/app-steam-379430.scope",
            None,
        );
        let task = TaskInfo {
            tid: 379430,
            process_pid: 379430,
            process_ppid: 1,
            comm: "KingdomCome".into(),
            process_comm: "KingdomCome".into(),
            process_starttime_ticks: Some(379430),
            task_starttime_ticks: Some(379430),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        };
        let profile = Profile {
            name: "game".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-1").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        assert_eq!(task.class, TaskClass::Game);
        assert!(profile_matches_task(&task, &profile));
    }

    #[test]
    fn profile_apply_summary_counts_matching_tasks_and_pending_changes() {
        let task_correct = TaskInfo {
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
            from_cgroup: false,
        };
        let task_pending = TaskInfo {
            tid: 8,
            process_pid: 8,
            process_ppid: 1,
            comm: "WorkerThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(80),
            task_starttime_ticks: Some(80),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        };
        let task_unmatched = TaskInfo {
            tid: 9,
            process_pid: 9,
            process_ppid: 1,
            comm: "Compositor".into(),
            process_comm: "sway".into(),
            process_starttime_ticks: Some(90),
            task_starttime_ticks: Some(90),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Compositor,
            sched_policy: None,
            from_cgroup: false,
        };
        let tasks = BTreeMap::from([(7, task_correct), (8, task_pending), (9, task_unmatched)]);
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-1").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        let summary = profile_apply_summary_with_reader(&tasks, &profile, |tid| match tid {
            7 => Ok(CpuMask::parse("0-1").unwrap()),
            8 => Ok(CpuMask::parse("0").unwrap()),
            9 => Ok(CpuMask::parse("0-1").unwrap()),
            other => panic!("unexpected TID {other}"),
        })
        .unwrap();

        assert_eq!(
            summary,
            ProfileApplySummary {
                checked_tasks: 2,
                pending_changes: 1,
            }
        );
    }

    #[test]
    fn profile_matched_task_count_counts_only_matching_rules() {
        let game_task = TaskInfo {
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
            from_cgroup: false,
        };
        let compositor_task = TaskInfo {
            tid: 8,
            process_pid: 8,
            process_ppid: 1,
            comm: "Compositor".into(),
            process_comm: "sway".into(),
            process_starttime_ticks: Some(80),
            task_starttime_ticks: Some(80),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Compositor,
            sched_policy: None,
            from_cgroup: false,
        };
        let tasks = BTreeMap::from([(7, game_task), (8, compositor_task)]);
        let profile = Profile {
            name: "game-render".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: vec![CompiledPattern::new("RenderThread".to_owned()).unwrap()],
            }],
        };

        assert_eq!(profile_matched_task_count(&tasks, &profile), 1);
        assert!(profile_matches_task(tasks.get(&7).unwrap(), &profile));
        assert!(!profile_matches_task(tasks.get(&8).unwrap(), &profile));
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
            from_cgroup: false,
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

    #[test]
    fn profile_offline_cpu_warnings_detects_rule_with_offline_cpus() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-3").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let online = CpuMask::parse("0-1").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rule_index, 0);
        assert_eq!(warnings[0].requested, "0-3");
        assert_eq!(warnings[0].online, "0-1");
    }

    #[test]
    fn profile_offline_cpu_warnings_empty_when_subset() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: CpuMask::parse("0-1").unwrap(),
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let online = CpuMask::parse("0-3").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert!(warnings.is_empty());
    }

    #[test]
    fn profile_offline_cpu_warnings_multiple_rules_report_correct_indexes() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![
                ProfileRule {
                    affinity: CpuMask::parse("0-1").unwrap(),
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                },
                ProfileRule {
                    affinity: CpuMask::parse("2-3").unwrap(),
                    match_class: vec![TaskClass::GameHelper],
                    match_comm: Vec::new(),
                },
            ],
        };
        let online = CpuMask::parse("0-1").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rule_index, 1);
        assert_eq!(warnings[0].requested, "2-3");
        assert_eq!(warnings[0].online, "0-1");
    }

    #[test]
    fn profile_rule_overlap_warnings_broad_game_before_specific_render_thread_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "0-7"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "2-5"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }

    #[test]
    fn profile_rule_overlap_warnings_disjoint_classes_do_not_warn() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "0-7"

            [[profile.rules]]
            match_class = ["Compositor"]
            affinity = "8-11"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert!(warnings.is_empty());
    }

    #[test]
    fn profile_rule_overlap_warnings_catch_all_before_anything_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            affinity = "0-7"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "2-5"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }

    #[test]
    fn profile_rule_overlap_warnings_exact_same_comm_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "0-3"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "4-7"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }
}
