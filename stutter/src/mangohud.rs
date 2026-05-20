use std::{fs, io::BufRead, path::Path};

use anyhow::Context;

use crate::recorder::{FrameEvent, monotonic_now_ns};

const FRAMETIME_HEADERS: &[&str] = &[
    "frametime",
    "frametime_ms",
    "frame_time",
    "frame_time_ms",
    "frame time",
    "frame time ms",
];

const ELAPSED_HEADERS: &[&str] = &[
    "elapsed_ms",
    "time_ms",
    "ms",
    "time",
    "elapsed",
    "elapsed_ns",
    "time_ns",
    "ns",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ElapsedUnit {
    Milliseconds,
    Nanoseconds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MangoHudCsvSchema {
    frametime_idx: usize,
    elapsed_idx: Option<usize>,
    elapsed_unit: ElapsedUnit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MangoHudCsvLayout {
    schema: MangoHudCsvSchema,
    data_start_offset: u64,
}

pub fn read_frame_events(
    path: &Path,
    ignore_offset: u64,
    alignment_monotonic_ns: Option<u64>,
    alignment_raw_elapsed_ms: Option<u64>,
    recorder_start_monotonic_ns: Option<u64>,
) -> anyhow::Result<Vec<FrameEvent>> {
    let layout = detect_layout(path)?;
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to read MangoHud log {}", path.display()))?;

    let seek_offset = ignore_offset.max(layout.data_start_offset);
    let mut skip_first_line = false;
    if seek_offset > layout.data_start_offset {
        use std::io::{Read, Seek, SeekFrom};
        // By default, if we are at a nonzero offset, we assume we might be mid-line
        // unless we can prove we are at a newline boundary.
        skip_first_line = true;
        if file.seek(SeekFrom::Start(seek_offset - 1)).is_ok() {
            let mut buf = [0u8; 1];
            if file.read_exact(&mut buf).is_ok() && buf[0] == b'\n' {
                skip_first_line = false;
            }
        }
        file.seek(SeekFrom::Start(seek_offset))?;
    } else {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(layout.data_start_offset))?;
    }

    let reader = std::io::BufReader::new(file);
    let mut lines = reader.lines();
    if skip_first_line {
        // Skip the first line if it's the header (ignore_offset == 0) or if
        // we sought into the middle of a row.
        let _ = lines.next();
    }

    parse_frame_events_with_schema(
        &layout.schema,
        lines,
        alignment_monotonic_ns,
        alignment_raw_elapsed_ms,
        recorder_start_monotonic_ns,
    )
}

#[cfg(test)]
fn parse_frame_events<I>(
    header: &str,
    lines: I,
    alignment_monotonic_ns: Option<u64>,
    alignment_raw_elapsed_ms: Option<u64>,
    recorder_start_monotonic_ns: Option<u64>,
) -> anyhow::Result<Vec<FrameEvent>>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    if header.trim().is_empty() {
        return Ok(Vec::new());
    }
    let headers = split_csv_line(header);
    let schema = schema_from_headers(&headers).ok_or_else(|| {
        anyhow::anyhow!(
            "MangoHud CSV did not contain a recognized frametime column; headers={headers:?}"
        )
    })?;

    parse_frame_events_with_schema(
        &schema,
        lines,
        alignment_monotonic_ns,
        alignment_raw_elapsed_ms,
        recorder_start_monotonic_ns,
    )
}

fn parse_frame_events_with_schema<I>(
    schema: &MangoHudCsvSchema,
    lines: I,
    alignment_monotonic_ns: Option<u64>,
    alignment_raw_elapsed_ms: Option<u64>,
    recorder_start_monotonic_ns: Option<u64>,
) -> anyhow::Result<Vec<FrameEvent>>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    let mut events = Vec::new();
    let mut first_elapsed_ms: Option<u64> = None;
    let mut accumulated_ms = 0.0;

    let alignment_recorder_elapsed_ms = alignment_monotonic_ns
        .zip(recorder_start_monotonic_ns)
        .map(|(m, r)| m.saturating_sub(r) / 1_000_000);

    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some((raw_val, frametime_ms)) = parse_frame_line(schema, &line) {
            if first_elapsed_ms.is_none() {
                first_elapsed_ms = raw_val;
            }

            let elapsed_ms = if let (Some(raw), Some(first_raw), Some(observed_ms)) = (
                raw_val,
                alignment_raw_elapsed_ms,
                alignment_recorder_elapsed_ms,
            ) {
                // Monotonic observed alignment
                observed_ms + raw.saturating_sub(first_raw)
            } else if let Some(raw) = raw_val {
                // Approximate alignment (relative to first row)
                let first = first_elapsed_ms.unwrap_or(0);
                raw.saturating_sub(first)
            } else if let Some(observed_ms) = alignment_recorder_elapsed_ms {
                // No raw elapsed column, but we have alignment from first row
                let val = observed_ms + accumulated_ms as u64;
                accumulated_ms += frametime_ms;
                val
            } else {
                // Fallback: zero-based accumulation
                let val = accumulated_ms as u64;
                accumulated_ms += frametime_ms;
                val
            };

            events.push(FrameEvent {
                elapsed_ms,
                frametime_ms,
            });
        }
    }

    Ok(events)
}

pub struct MangoHudLiveParser {
    schema: MangoHudCsvSchema,
}

impl MangoHudLiveParser {
    fn new(schema: MangoHudCsvSchema) -> Self {
        Self { schema }
    }

    pub fn parse_line(&mut self, line: &str) -> Option<FrameEvent> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        let (elapsed_ms, frametime_ms) = parse_frame_line(&self.schema, line)?;
        let elapsed_ms = elapsed_ms.unwrap_or(0);

        Some(FrameEvent {
            elapsed_ms,
            frametime_ms,
        })
    }
}

pub async fn tail_frames(
    path: std::path::PathBuf,
    start_offset: u64,
    tx: tokio::sync::mpsc::Sender<FrameEvent>,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

    let mut file = tokio::fs::File::open(&path).await.with_context(|| {
        format!(
            "failed to open MangoHud log for tailing: {}",
            path.display()
        )
    })?;
    file.seek(SeekFrom::Start(start_offset)).await?;

    let mut read_buf = vec![0_u8; 8192];
    let mut pending = String::new();
    let layout = detect_layout(&path)?;
    log::info!(
        "mangohud_schema_detected path={} frametime_idx={} elapsed_idx={:?} elapsed_unit={:?} data_start_offset={}",
        path.display(),
        layout.schema.frametime_idx,
        layout.schema.elapsed_idx,
        layout.schema.elapsed_unit,
        layout.data_start_offset
    );
    let mut parser = MangoHudLiveParser::new(layout.schema);

    loop {
        let n = match file.read(&mut read_buf).await {
            Ok(0) => {
                tokio::time::sleep(std::time::Duration::from_millis(75)).await;
                continue;
            }
            Ok(n) => n,
            Err(err) => {
                log::warn!(
                    "mangohud_tail_read_failed path={} err={err:#}",
                    path.display()
                );
                return Err(err.into());
            }
        };

        let chunk = String::from_utf8_lossy(&read_buf[..n]);
        pending.push_str(&chunk);

        while let Some(newline_pos) = pending.find('\n') {
            let mut line = pending[..newline_pos].to_string();

            if line.ends_with('\r') {
                line.pop();
            }

            pending.drain(..=newline_pos);

            if let Some(frame) = parser.parse_line(&line)
                && tx.try_send(frame).is_err()
            {
                // Channel full or receiver gone.
                // If receiver is gone, exiting is okay.
                if tx.is_closed() {
                    return Ok(());
                }
            }
        }
    }
}

pub async fn poll_alignment(path: &Path, start_offset: u64) -> anyhow::Result<(u64, u64)> {
    use std::io::{Read, Seek, SeekFrom};

    use tokio::time::{Duration, sleep};

    let mut layout_cache: Option<MangoHudCsvLayout> = None;

    loop {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                sleep(Duration::from_millis(500)).await;
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        let len = file.metadata()?.len();

        if layout_cache.is_none() {
            match try_detect_layout(path) {
                Ok(Some(layout)) => layout_cache = Some(layout),
                Ok(None) => {
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }

        let Some(layout) = layout_cache.as_ref() else {
            sleep(Duration::from_millis(500)).await;
            continue;
        };
        let schema = &layout.schema;
        let read_offset = start_offset.max(layout.data_start_offset);

        if len > read_offset {
            file.seek(SeekFrom::Start(read_offset))?;
            let mut reader = std::io::BufReader::new(file);

            let mut line = String::new();

            if read_offset > 0 {
                let mut f2 = fs::File::open(path)?;
                f2.seek(SeekFrom::Start(read_offset - 1))?;
                let mut b = [0u8; 1];
                if f2.read_exact(&mut b).is_ok() && b[0] != b'\n' {
                    reader.read_line(&mut line)?;
                }
            }

            line.clear();
            if reader.read_line(&mut line)? > 0 {
                let observed_ns = monotonic_now_ns().unwrap_or(0);
                if let Some((Some(raw_elapsed_ms), _)) = parse_frame_line(schema, &line) {
                    return Ok((raw_elapsed_ms, observed_ns));
                }

                return Ok((0, observed_ns));
            }
        }

        sleep(Duration::from_millis(500)).await;
    }
}

fn detect_layout(path: &Path) -> anyhow::Result<MangoHudCsvLayout> {
    try_detect_layout(path)?.with_context(|| {
        format!(
            "MangoHud CSV did not contain a recognized frame header: {}",
            path.display()
        )
    })
}

fn try_detect_layout(path: &Path) -> std::io::Result<Option<MangoHudCsvLayout>> {
    let file = fs::File::open(path)?;
    detect_layout_from_reader(std::io::BufReader::new(file))
}

fn detect_layout_from_reader<R: BufRead>(
    mut reader: R,
) -> std::io::Result<Option<MangoHudCsvLayout>> {
    let mut offset = 0_u64;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }

        let next_offset = offset + n as u64;
        let headers = split_csv_line(&line);
        if let Some(schema) = schema_from_headers(&headers) {
            return Ok(Some(MangoHudCsvLayout {
                schema,
                data_start_offset: next_offset,
            }));
        }

        offset = next_offset;
    }
}

fn schema_from_headers(headers: &[String]) -> Option<MangoHudCsvSchema> {
    let frametime_idx = find_header(headers, FRAMETIME_HEADERS)?;
    let elapsed_idx = find_header(headers, ELAPSED_HEADERS);
    let elapsed_unit = elapsed_idx
        .and_then(|idx| headers.get(idx))
        .map(|header| elapsed_unit_for_header(header))
        .unwrap_or(ElapsedUnit::Milliseconds);

    Some(MangoHudCsvSchema {
        frametime_idx,
        elapsed_idx,
        elapsed_unit,
    })
}

fn elapsed_unit_for_header(header: &str) -> ElapsedUnit {
    match header.trim().to_ascii_lowercase().as_str() {
        "elapsed" | "elapsed_ns" | "time_ns" | "ns" => ElapsedUnit::Nanoseconds,
        _ => ElapsedUnit::Milliseconds,
    }
}

fn parse_frame_line(schema: &MangoHudCsvSchema, line: &str) -> Option<(Option<u64>, f64)> {
    let columns = split_csv_line(line);
    let frametime_ms = columns
        .get(schema.frametime_idx)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())?;

    let elapsed_ms = schema
        .elapsed_idx
        .and_then(|idx| columns.get(idx))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|raw| elapsed_value_to_ms(raw, schema.elapsed_unit));

    Some((elapsed_ms, frametime_ms))
}

fn elapsed_value_to_ms(raw: f64, unit: ElapsedUnit) -> u64 {
    let raw = raw.max(0.0);
    match unit {
        ElapsedUnit::Milliseconds => raw as u64,
        ElapsedUnit::Nanoseconds => (raw / 1_000_000.0) as u64,
    }
}

fn find_header(headers: &[String], candidates: &[&str]) -> Option<usize> {
    headers.iter().position(|header| {
        let normalized = header.trim().to_ascii_lowercase();
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(ch),
        }
    }

    fields.push(field.trim().to_owned());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANGOHUD_WITH_METADATA: &str = "\
os,cpu,gpu,ram,kernel,driver,cpuscheduler\n\
'Gentoo Linux',Intel Core i5-10600K CPU @ 4.10GHz,Intel(R) UHD Graphics 630 (CML GT2),32148228,7.0.1-cachyos,,performance\n\
fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed\n\
49.9594,20.0163,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,39991331\n\
49.9079,20.0369,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,60029893\n";

    #[test]
    fn detects_mangohud_frame_header_after_metadata_rows() {
        let layout =
            detect_layout_from_reader(std::io::BufReader::new(MANGOHUD_WITH_METADATA.as_bytes()))
                .unwrap()
                .unwrap();

        assert_eq!(layout.schema.frametime_idx, 1);
        assert_eq!(layout.schema.elapsed_idx, Some(15));
        assert_eq!(layout.schema.elapsed_unit, ElapsedUnit::Nanoseconds);
        assert_eq!(
            layout.data_start_offset,
            MANGOHUD_WITH_METADATA
                .lines()
                .take(3)
                .map(|line| line.len() + 1)
                .sum::<usize>() as u64
        );
    }

    #[test]
    fn live_parser_uses_detected_schema_for_first_tailed_data_row() {
        let layout =
            detect_layout_from_reader(std::io::BufReader::new(MANGOHUD_WITH_METADATA.as_bytes()))
                .unwrap()
                .unwrap();
        let mut parser = MangoHudLiveParser::new(layout.schema);

        let first = parser
            .parse_line("49.9594,20.0163,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,39991331")
            .unwrap();
        let second = parser
            .parse_line("49.9079,20.0369,3.33333,0,0,44,0,950,0,0,0,10.039,2.04078,0,800,60029893")
            .unwrap();

        assert_eq!(first.elapsed_ms, 39);
        assert_eq!(first.frametime_ms, 20.0163);
        assert_eq!(second.elapsed_ms, 60);
        assert_eq!(second.frametime_ms, 20.0369);
    }

    #[test]
    fn read_frame_events_parses_mangohud_metadata_and_elapsed_nanoseconds() -> anyhow::Result<()> {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join(format!(
            "stutter_test_mangohud_metadata_{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir)?;
        let path = temp_dir.join("mangohud.csv");
        fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;

        let events = read_frame_events(&path, 0, None, None, None)?;

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].elapsed_ms, 0);
        assert_eq!(events[1].elapsed_ms, 21);
        assert_eq!(events[0].frametime_ms, 20.0163);
        assert_eq!(events[1].frametime_ms, 20.0369);

        fs::remove_dir_all(temp_dir).ok();
        Ok(())
    }

    #[test]
    fn read_frame_events_respects_offset_after_mangohud_metadata() -> anyhow::Result<()> {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join(format!(
            "stutter_test_mangohud_offset_{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir)?;
        let path = temp_dir.join("mangohud.csv");
        fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;
        let layout = detect_layout(&path)?;

        let events = read_frame_events(&path, layout.data_start_offset, None, None, None)?;
        assert_eq!(events.len(), 2);

        let row1_len = MANGOHUD_WITH_METADATA
            .lines()
            .nth(3)
            .expect("sample has first frame row")
            .len()
            + 1;
        let events = read_frame_events(
            &path,
            layout.data_start_offset + row1_len as u64,
            None,
            None,
            None,
        )?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frametime_ms, 20.0369);

        fs::remove_dir_all(temp_dir).ok();
        Ok(())
    }

    #[tokio::test]
    async fn poll_alignment_uses_mangohud_elapsed_nanoseconds() -> anyhow::Result<()> {
        use std::io::Write;

        let temp_dir = std::env::temp_dir().join(format!(
            "stutter_test_mangohud_alignment_{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir)?;
        let path = temp_dir.join("mangohud.csv");
        fs::File::create(&path)?.write_all(MANGOHUD_WITH_METADATA.as_bytes())?;
        let layout = detect_layout(&path)?;

        let (raw_ms, observed_ns) = poll_alignment(&path, layout.data_start_offset).await?;

        assert_eq!(raw_ms, 39);
        assert!(observed_ns > 0);

        fs::remove_dir_all(temp_dir).ok();
        Ok(())
    }

    #[test]
    fn parses_header_based_frametime_csv() {
        let header = "elapsed_ms,frametime_ms";
        let data = "10,16.7\n20,33.4\n";
        let events = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized
        assert_eq!(events[1].elapsed_ms, 10); // 20 - 10
        assert_eq!(events[1].frametime_ms, 33.4);
    }

    #[test]
    fn parses_quoted_csv_fields() {
        let header = "elapsed_ms,\"frame,time\",frametime_ms";
        let data = "10,\"ignored, value\",16.7\n";
        let events = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized
        assert_eq!(events[0].frametime_ms, 16.7);
    }

    #[test]
    fn skips_non_finite_frametimes() {
        let header = "elapsed_ms,frametime_ms";
        let data = "10,NaN\n20,inf\n30,16.7\n";
        let events = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized (30 - 30)
        assert_eq!(events[0].frametime_ms, 16.7);
    }

    #[test]
    fn read_frame_events_respects_newline_boundary_offset() -> anyhow::Result<()> {
        use std::io::Write;
        let temp_dir =
            std::env::temp_dir().join(format!("stutter_test_mangohud_{}", std::process::id()));
        fs::create_dir_all(&temp_dir)?;
        let path = temp_dir.join("test.csv");

        let header = "elapsed_ms,frametime_ms\n";
        let row1 = "10,16.7\n";
        let row2 = "20,33.4\n";

        let mut f = fs::File::create(&path)?;
        f.write_all(header.as_bytes())?;
        let offset_after_header = header.len() as u64;
        f.write_all(row1.as_bytes())?;
        let offset_after_row1 = offset_after_header + row1.len() as u64;
        f.write_all(row2.as_bytes())?;
        drop(f);

        // Case 1: ignore_offset = 0. Should skip header, read row1 and row2.
        let events = read_frame_events(&path, 0, None, None, None)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frametime_ms, 16.7);

        // Case 2: ignore_offset = offset_after_header.
        // offset_after_header-1 is '\n'.
        // Should NOT skip the first line (row1).
        let events = read_frame_events(&path, offset_after_header, None, None, None)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frametime_ms, 16.7);

        // Case 3: ignore_offset = offset_after_header + 2 (mid row1).
        // Should skip partial row1, read row2.
        let events = read_frame_events(&path, offset_after_header + 2, None, None, None)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frametime_ms, 33.4);

        // Case 4: ignore_offset = offset_after_row1.
        // offset_after_row1-1 is '\n'.
        // Should NOT skip row2.
        let events = read_frame_events(&path, offset_after_row1, None, None, None)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frametime_ms, 33.4);

        fs::remove_dir_all(temp_dir).ok();
        Ok(())
    }

    #[test]
    fn test_alignment_with_monotonic_observed() {
        let header = "elapsed_ms,frametime_ms";
        let data = "1000,16.7\n1016,16.7\n1033,16.7\n";

        let alignment_monotonic_ns = Some(1_420_000_000); // 1420ms
        let alignment_raw_elapsed_ms = Some(1000);
        let recorder_start_monotonic_ns = Some(1_000_000_000); // 1000ms
        // observed_ms = (1420 - 1000) = 420ms

        let events = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            alignment_monotonic_ns,
            alignment_raw_elapsed_ms,
            recorder_start_monotonic_ns,
        )
        .unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].elapsed_ms, 420);
        assert_eq!(events[1].elapsed_ms, 436);
        assert_eq!(events[2].elapsed_ms, 453);
    }

    #[test]
    fn test_alignment_missing_elapsed_column() {
        let header = "frametime_ms";
        let data = "16.7\n16.7\n16.7\n";

        let alignment_monotonic_ns = Some(1_420_000_000); // 1420ms
        let recorder_start_monotonic_ns = Some(1_000_000_000); // 1000ms
        // observed_ms = 420ms

        let events = parse_frame_events(
            header,
            data.lines().map(|s| Ok(s.to_owned())),
            alignment_monotonic_ns,
            None,
            recorder_start_monotonic_ns,
        )
        .unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].elapsed_ms, 420);
        assert_eq!(events[1].elapsed_ms, 436); // 420 + 16.7
        assert_eq!(events[2].elapsed_ms, 453); // 436 + 16.7
    }
}
