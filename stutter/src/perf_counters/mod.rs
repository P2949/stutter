mod group;
mod limits;
mod sample;
mod syscall;

use std::collections::{BTreeMap, BTreeSet};

use log::warn;
use stutter_core::ids::Tid;

pub(crate) use self::group::try_open_disabled_cycles_current_thread;
use self::{
    group::PerfCounterGroup,
    limits::{normalize_config, select_target_tids},
};
pub use self::{limits::CpuPerfConfig, sample::CpuPerfDelta};
use crate::{metrics::TaskStatsMap, process_tree::TaskMap};

pub struct CpuPerfSampler {
    config: CpuPerfConfig,
    groups: BTreeMap<Tid, PerfCounterGroup>,
    skipped_tasks: BTreeSet<Tid>,
    disabled_reason: Option<String>,
    last_error: Option<String>,
    total_read_errors: u64,
    total_open_errors: u64,
    total_samples: u64,
}

impl CpuPerfSampler {
    pub fn new(config: CpuPerfConfig) -> Self {
        Self {
            config: normalize_config(config),
            groups: BTreeMap::new(),
            skipped_tasks: BTreeSet::new(),
            disabled_reason: None,
            last_error: None,
            total_read_errors: 0,
            total_open_errors: 0,
            total_samples: 0,
        }
    }

    pub fn sync_targets(&mut self, active_targets: &TaskMap, stats_by_task: &TaskStatsMap) {
        if self.disabled_reason.is_some() {
            self.groups.clear();
            return;
        }

        let selected = select_target_tids(active_targets, stats_by_task, self.config.max_tasks);
        self.skipped_tasks = active_targets
            .keys()
            .copied()
            .filter(|tid| !selected.contains(tid))
            .collect();

        self.groups.retain(|tid, _| selected.contains(tid));

        for tid in selected {
            if self.groups.contains_key(&tid) {
                continue;
            }

            match PerfCounterGroup::open(tid, &self.config) {
                Ok(group) => {
                    self.groups.insert(tid, group);
                }
                Err(err) => {
                    self.total_open_errors = self.total_open_errors.saturating_add(1);
                    let message = format!("task {}: {}", tid, err.message);
                    self.last_error = Some(message.clone());

                    if err.is_permission_denied() {
                        let reason = "cpu_perf unavailable: perf_event_open denied; check CAP_PERFMON/CAP_SYS_ADMIN or perf_event_paranoid".to_owned();
                        warn!("{reason}");
                        self.disabled_reason = Some(reason.clone());
                        self.last_error = Some(reason);
                        self.groups.clear();
                        break;
                    }

                    if !err.is_task_gone() {
                        warn!("cpu_perf_open_failed {message}");
                    }
                }
            }
        }
    }

    pub fn sample_interval(&mut self) -> BTreeMap<Tid, CpuPerfDelta> {
        let mut deltas = BTreeMap::new();
        if let Some(reason) = &self.disabled_reason {
            self.last_error = Some(reason.clone());
            return deltas;
        }

        for (tid, group) in &mut self.groups {
            self.total_samples = self.total_samples.saturating_add(1);
            match group.sample_interval() {
                Ok(delta) => {
                    deltas.insert(*tid, delta);
                }
                Err(message) => {
                    self.total_read_errors = self.total_read_errors.saturating_add(1);
                    self.last_error = Some(format!("task {}: {}", tid, message));
                    deltas.insert(
                        *tid,
                        CpuPerfDelta {
                            unavailable_reason: Some(message),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        deltas
    }

    pub fn active_counter_tasks(&self) -> usize {
        self.groups.len()
    }

    pub fn skipped_counter_tasks(&self) -> usize {
        self.skipped_tasks.len()
    }

    pub fn total_read_errors(&self) -> u64 {
        self.total_read_errors
    }

    pub fn total_open_errors(&self) -> u64 {
        self.total_open_errors
    }

    pub fn total_samples(&self) -> u64 {
        self.total_samples
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error
            .as_deref()
            .or(self.disabled_reason.as_deref())
    }
}
