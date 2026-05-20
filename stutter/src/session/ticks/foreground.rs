use crate::{session::MonitorSession, session_events::MonitorEvent};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ForegroundTickContext;

fn foreground_stale_state(snapshot: &crate::foreground::ForegroundWindowSnapshot) -> bool {
    snapshot.stale_ms.is_some()
}

pub(crate) fn foreground_identity_changed(
    old: Option<&crate::foreground::ForegroundWindowSnapshot>,
    new: &crate::foreground::ForegroundWindowSnapshot,
) -> bool {
    let Some(old) = old else {
        return true;
    };

    old.source != new.source
        || old.status != new.status
        || old.pid != new.pid
        || old.app_id.as_deref() != new.app_id.as_deref()
        || old.class.as_deref() != new.class.as_deref()
        || old.window_id.as_deref() != new.window_id.as_deref()
        || old.workspace.as_deref() != new.workspace.as_deref()
        || foreground_stale_state(old) != foreground_stale_state(new)
}

impl MonitorSession {
    fn foreground_event_for_snapshot(
        &self,
        snapshot: &crate::foreground::ForegroundWindowSnapshot,
    ) -> Option<MonitorEvent> {
        snapshot
            .to_event(self.config.focus.foreground_include_title)
            .map(MonitorEvent::from)
    }
    pub(crate) async fn handle_foreground_tick(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let Some(resolver) = self.foreground_resolver.as_mut() else {
            return Ok(());
        };

        let snapshot = resolver.sample(elapsed_ms);
        let changed = foreground_identity_changed(self.current_foreground.as_ref(), &snapshot);

        if changed {
            if self.current_foreground.is_some() {
                self.foreground_switch_count = self.foreground_switch_count.saturating_add(1);
            }
            if let Some(event) = self.foreground_event_for_snapshot(&snapshot) {
                self.dispatch_monitor_event(event).await?;
            }
        }

        self.current_foreground = Some(snapshot);

        Ok(())
    }
}
