use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use log::info;

use crate::{
    community_rules::CommunityRulesDb,
    config::{FocusSource, ForegroundSource, model::MonitorConfig},
    error::ConfigError,
    focus::{FocusDecision, FocusPolicy, FocusResolver, ResolvedFocus},
    process_tree::{
        CompiledPattern, ProcessCache, TargetSnapshotInput, TaskFilters, find_auto_target_pids,
    },
    tasks::TaskTracker,
    watch::{
        WatchProcessConfig, WatchProcessState, add_watch_tree_pid, capture_tree_root_starttimes,
        process_root_starttime, remove_stale_tree_roots, remove_watch_tree_pid,
        resolve_watch_process,
    },
};

pub struct TargetPolicy {
    pub manual_pids: Vec<u32>,
    pub configured_tree_pids: Vec<u32>,
    pub cgroupv2: Option<PathBuf>,
    pub exclude_tree_pids: Vec<u32>,
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
    pub keep_missing_pid: bool,
    pub max_tasks: usize,
    pub compiled_filters: TaskFilters,
}

impl TargetPolicy {
    pub fn from_monitor_config(config: &MonitorConfig) -> Result<Self, ConfigError> {
        let compiled_filters =
            compile_target_filters(&config.target.include_comm, &config.target.exclude_comm)?;

        Ok(Self {
            manual_pids: config.target.target_pids.clone(),
            configured_tree_pids: config.target.tree_pids.clone(),
            cgroupv2: config.target.cgroupv2.clone(),
            exclude_tree_pids: config.target.exclude_tree_pids.clone(),
            include_comm: config.target.include_comm.clone(),
            exclude_comm: config.target.exclude_comm.clone(),
            keep_missing_pid: config.target.keep_missing_pid,
            max_tasks: config.target.max_tasks,
            compiled_filters,
        })
    }
}

fn compile_target_filters(
    include_comm: &[String],
    exclude_comm: &[String],
) -> Result<TaskFilters, ConfigError> {
    Ok(TaskFilters {
        include_comm: compile_target_patterns("include_comm", include_comm)?,
        exclude_comm: compile_target_patterns("exclude_comm", exclude_comm)?,
    })
}

fn compile_target_patterns(
    field: &'static str,
    patterns: &[String],
) -> Result<Vec<CompiledPattern>, ConfigError> {
    patterns
        .iter()
        .map(|pattern| {
            CompiledPattern::new(pattern.clone()).map_err(|source| {
                ConfigError::InvalidTargetFilter {
                    field,
                    pattern: pattern.clone(),
                    source,
                }
            })
        })
        .collect()
}

pub struct TargetController {
    pub policy: TargetPolicy,
    pub watch_config: WatchProcessConfig,
    pub dynamic_tree_pids: Vec<u32>,
    pub watch_state: WatchProcessState,
    pub tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub process_cache: ProcessCache,
    pub tasks: TaskTracker,
}

impl TargetController {
    pub fn new(
        policy: TargetPolicy,
        watch_config: WatchProcessConfig,
        dynamic_tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self {
            policy,
            watch_config,
            dynamic_tree_pids,
            watch_state,
            tree_root_starttimes,
            process_cache: ProcessCache::default(),
            tasks: TaskTracker::default(),
        }
    }

    pub fn from_policy_parts(
        policy: TargetPolicy,
        watch_config: WatchProcessConfig,
        dynamic_tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self::new(
            policy,
            watch_config,
            dynamic_tree_pids,
            watch_state,
            tree_root_starttimes,
        )
    }

    pub fn effective_tree_pids(&self) -> &[u32] {
        &self.dynamic_tree_pids
    }

    pub fn replace_dynamic_tree_roots(&mut self, roots: Vec<u32>) {
        self.dynamic_tree_pids = roots;
        self.tree_root_starttimes = capture_tree_root_starttimes(&self.dynamic_tree_pids);
    }

    pub fn clear_dynamic_tree_roots(&mut self) {
        self.dynamic_tree_pids.clear();
        self.tree_root_starttimes.clear();
    }

    pub fn add_watch_root(&mut self, pid: u32) {
        add_watch_tree_pid(&mut self.dynamic_tree_pids, pid);
        self.tree_root_starttimes
            .insert(pid, process_root_starttime(pid));
    }

    pub fn remove_watch_root(&mut self, pid: u32) {
        remove_watch_tree_pid(&mut self.dynamic_tree_pids, pid);
        self.tree_root_starttimes.remove(&pid);
    }

    pub fn remove_stale_dynamic_tree_roots(&mut self) -> Vec<u32> {
        remove_stale_tree_roots(
            &mut self.dynamic_tree_pids,
            &mut self.tree_root_starttimes,
            self.watch_state.running_pid(),
        )
    }

    pub fn has_tree_roots(&self) -> bool {
        !self.dynamic_tree_pids.is_empty()
    }

    pub fn target_snapshot_input<'a>(
        &'a mut self,
        community_rules: Option<&'a CommunityRulesDb>,
    ) -> TargetSnapshotInput<'a> {
        Self::target_snapshot_input_from_parts(
            &self.policy,
            &self.dynamic_tree_pids,
            &mut self.process_cache,
            community_rules,
        )
    }

    pub fn target_snapshot_input_from_parts<'a>(
        policy: &'a TargetPolicy,
        dynamic_tree_pids: &'a [u32],
        process_cache: &'a mut ProcessCache,
        community_rules: Option<&'a CommunityRulesDb>,
    ) -> TargetSnapshotInput<'a> {
        TargetSnapshotInput::default()
            .manual_pids(&policy.manual_pids)
            .tree_pids(dynamic_tree_pids)
            .cgroup_path(policy.cgroupv2.as_deref())
            .exclude_tree_pids(&policy.exclude_tree_pids)
            .filters(&policy.compiled_filters)
            .keep_missing_pid(policy.keep_missing_pid)
            .cache(process_cache)
            .community_rules(community_rules)
    }
}

pub(crate) fn needs_tree_tick_from_parts(
    had_tree_roots: bool,
    watch_process_active: bool,
    cgroupv2_active: bool,
) -> bool {
    had_tree_roots || watch_process_active || cgroupv2_active
}

pub(crate) fn tree_tick_interval_ms(config: &MonitorConfig) -> u64 {
    config.watch.poll_ms
}

fn foreground_capture_enabled(config: &MonitorConfig) -> bool {
    config.focus.foreground_window
        || (config.focus.auto_focus && config.focus.focus_source != FocusSource::Heuristic)
}

fn foreground_resolver_from_config(
    config: &MonitorConfig,
) -> crate::foreground::ForegroundResolver {
    let resolver = match config.focus.foreground_source {
        ForegroundSource::Auto => crate::foreground::auto_foreground_resolver(),
        ForegroundSource::Sway => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::SwayForegroundProvider::new(),
        )),
        ForegroundSource::Hyprland => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::HyprlandForegroundProvider::new(),
        )),
        ForegroundSource::Gnome => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::GnomeForegroundProvider::new(),
        )),
        ForegroundSource::Kde => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::KdeForegroundProvider::new(),
        )),
        ForegroundSource::X11 => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::X11ForegroundProvider::new(),
        )),
    };

    resolver
        .with_include_title(config.focus.foreground_include_title)
        .with_max_stale_ms(config.focus.foreground_max_stale_ms)
}

pub(crate) struct SessionTargetPlan {
    pub(crate) tree_pids: Vec<u32>,
    pub(crate) watch_config: WatchProcessConfig,
    pub(crate) watch_state: WatchProcessState,
    pub(crate) tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub(crate) had_tree_roots: bool,
    pub(crate) focus_resolver: Option<FocusResolver>,
    pub(crate) current_focus: Option<ResolvedFocus>,
    pub(crate) foreground_resolver: Option<crate::foreground::ForegroundResolver>,
    pub(crate) current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub(crate) community_rules: crate::community_rules::CommunityRulesStatus,
}

impl SessionTargetPlan {
    pub(crate) async fn resolve(config: &MonitorConfig) -> anyhow::Result<Self> {
        let explicit_target = config.has_explicit_target();
        let mut tree_pids = config.target.tree_pids.clone();

        let mut focus_resolver = None;
        let mut current_focus = None;
        let foreground_enabled = foreground_capture_enabled(config);
        let foreground_resolver =
            foreground_enabled.then(|| foreground_resolver_from_config(config));
        let current_foreground = None;

        let user_config = crate::config_file::load_user_config()?;

        log::info!(
            "monitor_session_config source=monitor_config summary_period_ms={} spike_threshold_ns={} max_tasks={} hwmon={} cpu_freq={} foreground_window={} focus_source={:?} foreground_source={:?}",
            config.timing.summary_period_ms,
            config.timing.spike_threshold_ns,
            config.target.max_tasks,
            config.probes.hwmon,
            config.probes.cpu_freq,
            config.focus.foreground_window,
            config.focus.focus_source,
            config.focus.foreground_source,
        );

        let community_rules_config =
            crate::config_file::community_rules_config_from_user_config(user_config.as_ref());
        let community_rules =
            crate::community_rules::load_community_rules_status(&community_rules_config);
        let community_rules_status = community_rules.label();
        match &community_rules {
            crate::community_rules::CommunityRulesStatus::Loaded { db } => {
                log::info!(
                    "community_rules_status status={} rules={}",
                    community_rules_status,
                    db.rule_count()
                );
            }
            crate::community_rules::CommunityRulesStatus::Disabled => {
                log::info!("community_rules_status status={community_rules_status}");
            }
            crate::community_rules::CommunityRulesStatus::Failed { error } => {
                log::warn!("community_rules_status status={community_rules_status} err={error}");
            }
        }

        if !explicit_target && config.focus.auto_focus {
            let policy = FocusPolicy {
                poll_ms: config.focus.auto_focus_poll_ms,
                min_confidence: config.focus.auto_focus_min_confidence,
                switch_margin: config.focus.auto_focus_switch_margin,
                switch_cooldown_ms: config.focus.auto_focus_switch_cooldown_ms,
                required_winner_polls: config.focus.auto_focus_required_polls,
                max_roots: config.focus.auto_focus_max_roots,
            };

            let mut resolver = FocusResolver::new(policy);
            match resolver.sample(Path::new("/proc"), 0, None, FocusSource::Heuristic) {
                FocusDecision::Switch { new, .. } | FocusDecision::Keep { focus: new } => {
                    tree_pids = new.group.root_pids.clone();
                    info!(
                        "auto_focus_initial_target kind={:?} score={:.3} confidence={:.3} roots={:?} situation={:?}",
                        new.group.kind,
                        new.group.score,
                        new.group.confidence,
                        new.group.root_pids,
                        new.situation
                    );
                    current_focus = Some(new);
                }
                FocusDecision::NoTarget { reason } | FocusDecision::Clear { reason, .. } => {
                    info!("auto_focus_no_initial_target reason={reason}");
                }
            }

            focus_resolver = Some(resolver);
        } else if !explicit_target {
            let auto_targets = find_auto_target_pids(Path::new("/proc"));
            if auto_targets.is_empty() {
                anyhow::bail!(
                    "no target specified and no game launcher (gamescope, pressure-vessel, etc.) detected. \
                     Please provide --pid <PID>, --tree-pid <PID>, --watch-process <COMM>, or --cgroupv2 <PATH>"
                );
            }

            let pids: Vec<_> = auto_targets.iter().map(|(p, _)| *p).collect();
            let class = auto_targets[0].1;
            info!("auto_detected_launcher class={class} pids={pids:?}");
            let stdout_is_machine_stream =
                config.outputs.json_stream || config.csv_streams_to_stdout();
            if !stdout_is_machine_stream {
                println!(
                    "auto-detected game launcher: {class} (PIDs {pids:?}). monitoring tree..."
                );
            }
            tree_pids = pids;
        }

        let watch_config = WatchProcessConfig::from_monitor_config(config);
        let watch_state = match resolve_watch_process(&watch_config, &mut tree_pids).await? {
            Some(pid) => WatchProcessState::Running(pid),
            None => WatchProcessState::None,
        };

        let had_tree_roots = !tree_pids.is_empty();
        let tree_root_starttimes = capture_tree_root_starttimes(&tree_pids);

        Ok(Self {
            tree_pids,
            watch_config,
            watch_state,
            tree_root_starttimes,
            had_tree_roots,
            focus_resolver,
            current_focus,
            foreground_resolver,
            current_foreground,
            community_rules,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_policy_rejects_invalid_include_comm_regex() {
        let mut config = MonitorConfig::default();
        config.target.include_comm = vec!["/[unclosed/".to_owned()];

        let result = TargetPolicy::from_monitor_config(&config);

        match result {
            Err(ConfigError::InvalidTargetFilter { field, pattern, .. }) => {
                assert_eq!(field, "include_comm");
                assert_eq!(pattern, "/[unclosed/");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("expected invalid include_comm regex to fail"),
        }
    }
}
