use super::*;

impl MonitorSession {
    pub(crate) async fn refresh_tasks_and_emit_snapshot(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let previous_active_targets: BTreeSet<u32> = self
            .runtime
            .targeting
            .tasks
            .active_targets
            .keys()
            .map(|tid| tid.as_u32())
            .collect();

        self.refresh_tasks().await?;

        let removed_targets = previous_active_targets
            .into_iter()
            .filter(|tid| {
                !self
                    .runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            })
            .collect::<Vec<_>>();

        self.dispatch_monitor_event(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: self.runtime.targeting.tasks.active_targets.clone(),
            removed_targets,
        })
        .await?;

        Ok(())
    }

    pub(crate) async fn handle_target_tick(
        &mut self,
        context: TargetTickContext,
    ) -> anyhow::Result<Option<String>> {
        match context.event {
            TargetTickEvent::Tree => self.handle_tree_tick().await,
            TargetTickEvent::Watch => {
                self.handle_watch_tick().await?;
                Ok(None)
            }
        }
    }

    pub async fn handle_tree_tick(&mut self) -> anyhow::Result<Option<String>> {
        let mut should_exit = None;

        if let Some(root_pid) = self.runtime.targeting.watch_state.running_pid()
            && tree_root_is_stale(root_pid, &self.runtime.targeting.tree_root_starttimes)
        {
            self.runtime.targeting.remove_watch_root(root_pid);

            if !self.config.target.persistent {
                should_exit = Some("watched_process_exit".to_owned());
            } else {
                self.runtime.targeting.watch_state = WatchProcessState::Waiting;
                info!("watch_process_waiting_for_relaunch");
            }
        } else {
            let removed_roots = self.runtime.targeting.remove_stale_dynamic_tree_roots();

            for root in &removed_roots {
                info!("tree_root_removed pid={root}");
            }

            if !removed_roots.is_empty()
                && self.had_tree_roots
                && self.runtime.targeting.effective_tree_pids().is_empty()
                && !matches!(
                    self.runtime.targeting.watch_state,
                    WatchProcessState::Waiting
                )
            {
                should_exit = Some("tree_root_exit".to_owned());
            }
        }

        self.refresh_tasks_and_emit_snapshot().await?;

        // Belt-and-suspenders cleanup in case a refresh path exits before
        // emitting per-task removal diffs.
        self.runtime
            .targeting
            .tasks
            .prev_faults_snapshot
            .retain(|tid, _| {
                self.runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            });

        Ok(should_exit)
    }

    pub async fn handle_watch_tick(&mut self) -> anyhow::Result<()> {
        let Some(pattern) = self.runtime.targeting.watch_config.pattern.clone() else {
            return Ok(());
        };

        if !self.runtime.targeting.watch_state.should_poll() {
            return Ok(());
        }

        if let Some(pid) = find_process_by_pattern_at_with_cache(
            Path::new("/proc"),
            &pattern,
            &mut self.runtime.targeting.process_cache,
        ) {
            self.runtime.targeting.add_watch_root(pid);
            self.runtime.targeting.watch_state = WatchProcessState::Running(pid);
            info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

            self.refresh_tasks_and_emit_snapshot().await?;
        }

        Ok(())
    }

    pub async fn refresh_tasks(&mut self) -> anyhow::Result<()> {
        let targeting = &mut self.runtime.targeting;
        let policy = &targeting.policy;
        let dynamic_tree_pids = &targeting.dynamic_tree_pids;
        let process_cache = &mut targeting.process_cache;
        let tasks = &mut targeting.tasks;
        let target_snapshot_input = TargetController::target_snapshot_input_from_parts(
            policy,
            dynamic_tree_pids,
            process_cache,
            self.community_rules.as_db(),
        );

        let recording_started = crate::recorder!(self.handles)
            .run
            .as_ref()
            .map(|run| run.started_instant);
        let budget_report = tasks
            .refresh(crate::tasks::RefreshInput {
                target_snapshot_input,
                max_tasks: policy.max_tasks,
                tree_events: &mut crate::recorder_mut!(self.handles).buffers.tree_events,
                target_pid_map: &mut self.handles.ebpf.loaded.target_pid_map,
                prev_faults_map: self.handles.ebpf.loaded.prev_faults_map.as_mut(),
                elapsed_ms: self.started.elapsed().as_millis() as u64,
                recording_started,
            })
            .await?;

        if budget_report.scan_timed_out {
            crate::recorder_mut!(self.handles)
                .counters
                .process_scan_budget_exceeded_count += 1;
        }

        crate::recorder_mut!(self.handles)
            .counters
            .thread_scan_limited_count += budget_report.processes_thread_limited as u64;

        if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
            sampler.sync_targets(
                &self.runtime.targeting.tasks.active_targets,
                &self.runtime.targeting.tasks.stats_by_task,
            );
        }

        Ok(())
    }
}
