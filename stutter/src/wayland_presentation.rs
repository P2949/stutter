use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::recorder::WaylandPresentationEventRecord;

#[derive(Debug)]
pub struct WaylandPresentationLogReader {
    path: PathBuf,
    offset: u64,
    pending: String,
}

impl WaylandPresentationLogReader {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            offset: 0,
            pending: String::new(),
        })
    }

    pub fn open_tail(path: &Path) -> Result<Self> {
        let offset = fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            offset,
            pending: String::new(),
        })
    }

    pub fn read_new_events(&mut self) -> Result<Vec<WaylandPresentationEventRecord>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to open {}", self.path.display()));
            }
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.pending.clear();
        }

        file.seek(SeekFrom::Start(self.offset))?;
        let mut chunk = String::new();
        file.read_to_string(&mut chunk)?;
        self.offset = file.stream_position()?;
        if chunk.is_empty() {
            return Ok(Vec::new());
        }

        let mut combined = String::new();
        combined.push_str(&self.pending);
        combined.push_str(&chunk);
        self.pending.clear();

        let complete = if combined.ends_with('\n') {
            combined.as_str()
        } else if let Some((head, tail)) = combined.rsplit_once('\n') {
            self.pending = tail.to_owned();
            head
        } else {
            self.pending = combined;
            return Ok(Vec::new());
        };

        complete
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(parse_wayland_presentation_log_line)
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct WaylandPresentationLogLine {
    #[serde(default)]
    elapsed_ms: Option<u64>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    surface_role: Option<String>,
    #[serde(default)]
    commit_ns: Option<u64>,
    #[serde(default)]
    presented_ns: Option<u64>,
    #[serde(default)]
    commit_to_present_ns: Option<u64>,
    #[serde(default)]
    output_name: Option<String>,
    #[serde(default)]
    refresh_ns: Option<u64>,
    #[serde(default)]
    sequence: Option<u64>,
    #[serde(default)]
    zero_copy: Option<bool>,
    #[serde(default)]
    discarded: Option<bool>,
    #[serde(default)]
    flags: Option<Vec<String>>,
    #[serde(default)]
    confidence: Option<String>,
}

fn parse_wayland_presentation_log_line(line: &str) -> Result<WaylandPresentationEventRecord> {
    let parsed: WaylandPresentationLogLine =
        serde_json::from_str(line).with_context(|| "invalid Wayland presentation NDJSON line")?;
    let commit_to_present_ns = parsed.commit_to_present_ns.or_else(|| {
        parsed
            .presented_ns
            .zip(parsed.commit_ns)
            .and_then(|(presented, commit)| presented.checked_sub(commit))
    });
    let confidence = parsed.confidence.unwrap_or_else(|| {
        if parsed.discarded.unwrap_or(false) {
            "low"
        } else if commit_to_present_ns.is_some() {
            "high"
        } else {
            "low"
        }
        .to_owned()
    });

    Ok(WaylandPresentationEventRecord {
        elapsed_ms: parsed.elapsed_ms.unwrap_or(0),
        source: parsed.source.unwrap_or_else(|| "external_log".to_owned()),
        app_id: parsed.app_id,
        surface_role: parsed.surface_role,
        commit_ns: parsed.commit_ns,
        presented_ns: parsed.presented_ns,
        commit_to_present_ns,
        output_name: parsed.output_name,
        refresh_ns: parsed.refresh_ns,
        sequence: parsed.sequence,
        zero_copy: parsed.zero_copy,
        discarded: parsed.discarded.unwrap_or(false),
        flags: parsed.flags.unwrap_or_default(),
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_parses_ndjson_and_derives_duration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("presentation.ndjson");
        fs::write(
            &path,
            r#"{"commit_ns":1000,"presented_ns":2500,"output_name":"DP-1","zero_copy":true,"source":"gamescope"}"#,
        )
        .unwrap();
        fs::write(&path, fs::read_to_string(&path).unwrap() + "\n").unwrap();

        let mut reader = WaylandPresentationLogReader::open(&path).unwrap();
        let events = reader.read_new_events().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "gamescope");
        assert_eq!(events[0].commit_to_present_ns, Some(1500));
        assert_eq!(events[0].zero_copy, Some(true));
    }

    #[test]
    fn reader_holds_partial_lines_until_newline() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("presentation.ndjson");
        fs::write(&path, r#"{"commit_ns":1000"#).unwrap();

        let mut reader = WaylandPresentationLogReader::open(&path).unwrap();
        assert!(reader.read_new_events().unwrap().is_empty());

        fs::write(
            &path,
            r#"{"commit_ns":1000,"presented_ns":2000}
"#,
        )
        .unwrap();
        let events = reader.read_new_events().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].commit_to_present_ns, Some(1000));
    }
}
