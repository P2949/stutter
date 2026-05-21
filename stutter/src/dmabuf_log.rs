//! Cooperative DMABUF path log ingestion.
//!
//! Owns tailing and parsing external NDJSON records that describe buffer format/modifier,
//! allocation/import GPUs, and copy/scanout hints. It does not infer kernel-only DMABUF state.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::recorder::DmaBufEventRecord;

#[derive(Debug)]
pub struct DmaBufLogReader {
    path: PathBuf,
    offset: u64,
    pending: String,
}

impl DmaBufLogReader {
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

    pub fn read_new_events(&mut self) -> Result<Vec<DmaBufEventRecord>> {
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
            .map(parse_dmabuf_log_line)
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct DmaBufLogLine {
    #[serde(default)]
    elapsed_ms: Option<u64>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    surface_role: Option<String>,
    #[serde(default)]
    output_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    modifier: Option<String>,
    #[serde(default)]
    modifier_name: Option<String>,
    #[serde(default)]
    planes: Option<u32>,
    #[serde(default)]
    allocation_driver: Option<String>,
    #[serde(default)]
    import_driver: Option<String>,
    #[serde(default)]
    allocation_card: Option<String>,
    #[serde(default)]
    import_card: Option<String>,
    #[serde(default)]
    linear: Option<bool>,
    #[serde(default)]
    scanout_capable: Option<bool>,
    #[serde(default)]
    zero_copy: Option<bool>,
    #[serde(default)]
    explicit_sync: Option<bool>,
    #[serde(default)]
    copy_required: Option<bool>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
}

fn parse_dmabuf_log_line(line: &str) -> Result<DmaBufEventRecord> {
    let parsed: DmaBufLogLine =
        serde_json::from_str(line).with_context(|| "invalid DMABUF NDJSON line")?;
    let confidence = parsed.confidence.unwrap_or_else(|| {
        if parsed.copy_required.is_some()
            || parsed.scanout_capable.is_some()
            || parsed.zero_copy.is_some()
        {
            "medium"
        } else {
            "low"
        }
        .to_owned()
    });

    Ok(DmaBufEventRecord {
        elapsed_ms: parsed.elapsed_ms.unwrap_or(0),
        source: parsed.source.unwrap_or_else(|| "external_log".to_owned()),
        app_id: parsed.app_id,
        surface_role: parsed.surface_role,
        output_name: parsed.output_name,
        width: parsed.width,
        height: parsed.height,
        format: parsed.format,
        modifier: parsed.modifier,
        modifier_name: parsed.modifier_name,
        planes: parsed.planes,
        allocation_driver: parsed.allocation_driver,
        import_driver: parsed.import_driver,
        allocation_card: parsed.allocation_card,
        import_card: parsed.import_card,
        linear: parsed.linear,
        scanout_capable: parsed.scanout_capable,
        zero_copy: parsed.zero_copy,
        explicit_sync: parsed.explicit_sync,
        copy_required: parsed.copy_required,
        reason: parsed.reason,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_parses_ndjson_dmabuf_path_event() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dmabuf.ndjson");
        fs::write(
            &path,
            r#"{"elapsed_ms":1000,"source":"gamescope","surface_role":"game","format":"XRGB8888","modifier":"LINEAR","allocation_driver":"amdgpu","import_driver":"i915","scanout_capable":false,"copy_required":true,"reason":"modifier_mismatch"}"#,
        )
        .unwrap();
        fs::write(&path, fs::read_to_string(&path).unwrap() + "\n").unwrap();

        let mut reader = DmaBufLogReader::open(&path).unwrap();
        let events = reader.read_new_events().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "gamescope");
        assert_eq!(events[0].surface_role.as_deref(), Some("game"));
        assert_eq!(events[0].modifier.as_deref(), Some("LINEAR"));
        assert_eq!(events[0].allocation_driver.as_deref(), Some("amdgpu"));
        assert_eq!(events[0].import_driver.as_deref(), Some("i915"));
        assert_eq!(events[0].copy_required, Some(true));
        assert_eq!(events[0].reason.as_deref(), Some("modifier_mismatch"));
        assert_eq!(events[0].confidence, "medium");
    }

    #[test]
    fn reader_holds_partial_lines_until_newline() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dmabuf.ndjson");
        fs::write(&path, r#"{"format":"XRGB8888""#).unwrap();

        let mut reader = DmaBufLogReader::open(&path).unwrap();
        assert!(reader.read_new_events().unwrap().is_empty());

        fs::write(&path, r#"{"format":"XRGB8888","modifier":"LINEAR"}"#).unwrap();
        fs::write(&path, fs::read_to_string(&path).unwrap() + "\n").unwrap();
        let events = reader.read_new_events().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].format.as_deref(), Some("XRGB8888"));
    }
}
