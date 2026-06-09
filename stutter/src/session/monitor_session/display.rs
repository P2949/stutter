use super::*;

impl MonitorSession {
    pub(crate) fn handle_ui_tick(&mut self, context: UiTickContext) -> Option<String> {
        self.handle_tui_event(context.event)
    }

    pub fn handle_tui_event(&mut self, event: Event) -> Option<String> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => return Some("quit".to_owned()),
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.runtime.ui.tui_state.paused = !self.runtime.ui.tui_state.paused;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.runtime.ui.tui_state.sort_field =
                        self.runtime.ui.tui_state.sort_field.next();
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.runtime.ui.tui_state.next_filter_class();
                }
                _ => {}
            }
        }
        None
    }
}

pub(crate) fn display_driver_from_source(source: &str) -> Option<String> {
    match source {
        "amdgpu" | "amdgpu_tracepoint" => Some("amdgpu".to_owned()),
        "i915" | "i915_tracepoint" => Some("i915".to_owned()),
        _ => None,
    }
}
