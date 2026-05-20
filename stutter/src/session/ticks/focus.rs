use std::path::Path;

use log::info;

use crate::{
    focus::{FocusDecision, ResolvedFocus},
    session::MonitorSession,
    session_events::MonitorEvent,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct FocusTickContext;

impl MonitorSession {
    async fn emit_focus_changed(
        &mut self,
        elapsed_ms: u64,
        old: Option<&ResolvedFocus>,
        new: &ResolvedFocus,
    ) -> anyhow::Result<()> {
        info!(
            "auto_focus_changed elapsed_ms={} old_kind={:?} new_kind={:?} score={:.3} confidence={:.3} roots={:?} situation={:?}",
            elapsed_ms,
            old.map(|focus| focus.group.kind),
            new.group.kind,
            new.group.score,
            new.group.confidence,
            new.group.root_pids,
            new.situation
        );

        let event = MonitorEvent::FocusChanged {
            elapsed_ms,
            old_kind: old.map(|focus| focus.group.kind),
            new_kind: new.group.kind,
            root_pids: new.group.root_pids.clone(),
            member_pids: new.group.member_pids.clone(),
            confidence: new.group.confidence,
            score: new.group.score,
            situation: new.situation,
            reasons: new.group.reasons.clone(),
        };

        self.dispatch_monitor_event(event).await?;

        Ok(())
    }

    async fn emit_focus_cleared(
        &mut self,
        elapsed_ms: u64,
        old: Option<&ResolvedFocus>,
        reason: String,
    ) -> anyhow::Result<()> {
        info!(
            "auto_focus_cleared elapsed_ms={} old_kind={:?} reason={}",
            elapsed_ms,
            old.map(|focus| focus.group.kind),
            reason
        );

        let event = MonitorEvent::FocusCleared {
            elapsed_ms,
            old_kind: old.map(|focus| focus.group.kind),
            reason,
        };

        self.dispatch_monitor_event(event).await?;

        Ok(())
    }

    pub(crate) async fn handle_focus_tick(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let Some(resolver) = self.focus_resolver.as_mut() else {
            return Ok(());
        };

        let foreground = self.current_foreground.clone();
        let decision = resolver.sample(
            Path::new("/proc"),
            elapsed_ms,
            foreground.as_ref(),
            self.config.focus.focus_source,
        );

        match decision {
            FocusDecision::Switch { old, new } => {
                self.runtime
                    .targeting
                    .replace_dynamic_tree_roots(new.group.root_pids.clone());
                self.had_tree_roots = self.runtime.targeting.has_tree_roots();
                self.current_focus = Some(new.clone());
                self.focus_switch_count = self.focus_switch_count.saturating_add(1);
                self.refresh_tasks_and_emit_snapshot().await?;
                self.emit_focus_changed(elapsed_ms, old.as_ref(), &new)
                    .await?;
            }
            FocusDecision::Clear { old, reason } => {
                self.runtime.targeting.clear_dynamic_tree_roots();
                self.had_tree_roots = false;
                self.current_focus = None;
                self.refresh_tasks_and_emit_snapshot().await?;
                self.emit_focus_cleared(elapsed_ms, old.as_ref(), reason)
                    .await?;
            }
            FocusDecision::Keep { focus } => {
                self.current_focus = Some(focus);
            }
            FocusDecision::NoTarget { .. } => {}
        }

        Ok(())
    }
}
