use std::{
    fs::{self, OpenOptions},
    io::{BufRead, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::ActionId;

#[derive(Debug, Clone)]
pub struct AuditCommandInput {
    pub path: Option<PathBuf>,
    pub tail: usize,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub schema_version: u32,
    pub unix_nanos: u128,
    pub command: String,
    pub action_id: Option<ActionId>,
    pub safety_class: Option<crate::actions::SafetyClass>,
    pub dry_run: bool,
    pub success: bool,
    pub affected_tasks: usize,
    pub restore_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_phase: Option<crate::actions::ActionPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<String>,
    pub message: String,
}

impl AuditEvent {
    pub fn validate_identity_strings(&self) -> Result<(), stutter_core::ids::EmptyStringIdError> {
        if let Some(action_id) = &self.action_id {
            action_id.validate_non_empty()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn new(command: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            unix_nanos: unix_nanos_now(),
            command: command.into(),
            action_id: None,
            safety_class: None,
            dry_run: false,
            success: true,
            affected_tasks: 0,
            restore_path: None,
            action_phase: None,
            error_category: None,
            message: message.into(),
        }
    }
}

pub fn default_audit_log_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("audit");
    path.push("actions.jsonl");
    path
}

pub fn append_audit_event(event: &AuditEvent) -> anyhow::Result<()> {
    append_audit_event_to_path(&default_audit_log_path(), event)
}

pub fn append_audit_event_to_path(path: &Path, event: &AuditEvent) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open audit log {}", path.display()))?;
    serde_json::to_writer(&mut file, event)
        .with_context(|| format!("failed to write audit event {}", path.display()))?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn read_audit_tail(path: &Path, tail: usize) -> anyhow::Result<Vec<AuditEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open audit log {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<AuditEvent>(&line)
            .with_context(|| format!("failed to parse audit log {}", path.display()))?;
        event
            .validate_identity_strings()
            .with_context(|| format!("invalid audit event action_id in {}", path.display()))?;
        events.push(event);
    }
    if events.len() > tail {
        Ok(events.split_off(events.len() - tail))
    } else {
        Ok(events)
    }
}

pub fn audit_command(input: AuditCommandInput) -> anyhow::Result<()> {
    let path = input.path.unwrap_or_else(default_audit_log_path);
    let events = read_audit_tail(&path, input.tail)?;
    if input.json {
        println!("{}", serde_json::to_string_pretty(&events)?);
    } else if events.is_empty() && !path.exists() {
        println!("no audit log found at {}", path.display());
    } else if events.is_empty() {
        println!("audit log {} has no events", path.display());
    } else {
        print!("{}", render_audit_events(&events));
    }
    Ok(())
}

pub fn render_audit_events(events: &[AuditEvent]) -> String {
    let mut out = String::new();
    for event in events {
        out.push_str(&format!(
            "time={} command={} success={} dry_run={} affected_tasks={} action={} phase={} category={} message={}\n",
            event.unix_nanos,
            event.command,
            event.success,
            event.dry_run,
            event.affected_tasks,
            event.action_id.as_ref().map(ActionId::as_str).unwrap_or("-"),
            event
                .action_phase
                .map(crate::actions::ActionPhase::as_str)
                .unwrap_or("-"),
            event.error_category.as_deref().unwrap_or("-"),
            event.message
        ));
    }
    out
}

pub fn audit_or_warn(event: &AuditEvent) {
    if let Err(err) = append_audit_event(event) {
        log::warn!("audit_write_failed err={err:#}");
    }
}

pub fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-audit-test-{name}-{}-{}",
            std::process::id(),
            unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn event(message: &str) -> AuditEvent {
        AuditEvent::new("test", message)
    }

    #[test]
    fn append_writes_valid_ndjson() {
        let dir = temp_dir("append");
        let path = dir.join("actions.jsonl");

        append_audit_event_to_path(&path, &event("one")).unwrap();

        let data = fs::read_to_string(&path).unwrap();
        let lines = data.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let parsed: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.message, "one");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audit_event_typed_action_id_keeps_json_shape() {
        let mut event = event("typed");
        event.action_id = Some(ActionId::new("profile-restore"));

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(value["action_id"], "profile-restore");
    }

    #[test]
    fn read_audit_tail_rejects_empty_action_id() {
        let dir = temp_dir("empty-action-id");
        let path = dir.join("actions.jsonl");
        let json = serde_json::json!({
            "schema_version": 1,
            "unix_nanos": 1,
            "command": "test",
            "action_id": "",
            "safety_class": null,
            "dry_run": false,
            "success": true,
            "affected_tasks": 0,
            "restore_path": null,
            "message": "bad"
        });
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&json).unwrap()),
        )
        .unwrap();

        let err = read_audit_tail(&path, 10).expect_err("empty action_id should be rejected");

        assert!(
            format!("{err:#}").contains("ActionId cannot be empty"),
            "unexpected error: {err:#}"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audit_tail_returns_only_last_n_events() {
        let dir = temp_dir("tail");
        let path = dir.join("actions.jsonl");
        append_audit_event_to_path(&path, &event("one")).unwrap();
        append_audit_event_to_path(&path, &event("two")).unwrap();
        append_audit_event_to_path(&path, &event("three")).unwrap();

        let events = read_audit_tail(&path, 2).unwrap();

        assert_eq!(
            events
                .iter()
                .map(|event| event.message.as_str())
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_audit_log_is_not_an_error() {
        let dir = temp_dir("missing");
        let events = read_audit_tail(&dir.join("missing.jsonl"), 20).unwrap();

        assert!(events.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_mode_emits_parseable_array() {
        let events = vec![event("one"), event("two")];
        let json = serde_json::to_string(&events).unwrap();
        let parsed: Vec<AuditEvent> = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.len(), 2);
    }
}
