use std::{fs, io::BufRead, path::Path};

use anyhow::Context;

use crate::recorder::FrameEvent;

pub fn read_frame_events(path: &Path, ignore_offset: u64) -> anyhow::Result<Vec<FrameEvent>> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to read MangoHud log {}", path.display()))?;

    let mut header = String::new();
    {
        let mut reader = std::io::BufReader::new(&file);
        reader.read_line(&mut header)?;
    }

    let mut skip_first_line = ignore_offset == 0;
    if ignore_offset > 0 {
        use std::io::{Read, Seek, SeekFrom};
        // By default, if we are at a nonzero offset, we assume we might be mid-line
        // unless we can prove we are at a newline boundary.
        skip_first_line = true;
        if file.seek(SeekFrom::Start(ignore_offset - 1)).is_ok() {
            let mut buf = [0u8; 1];
            if file.read_exact(&mut buf).is_ok() && buf[0] == b'\n' {
                skip_first_line = false;
            }
        }
        file.seek(SeekFrom::Start(ignore_offset))?;
    } else {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0))?;
    }

    let reader = std::io::BufReader::new(file);
    let mut lines = reader.lines();
    if skip_first_line {
        // Skip the first line if it's the header (ignore_offset == 0) or if
        // we sought into the middle of a row.
        let _ = lines.next();
    }

    parse_frame_events(&header, lines)
}

pub fn parse_frame_events<I>(header: &str, lines: I) -> anyhow::Result<Vec<FrameEvent>>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    if header.trim().is_empty() {
        return Ok(Vec::new());
    }
    let headers = split_csv_line(header);
    let elapsed_idx = find_header(&headers, &["elapsed_ms", "time_ms", "ms"]);
    let frametime_idx = find_header(&headers, &["frametime", "frametime_ms", "frame_time"]);

    let mut events = Vec::new();
    let mut first_elapsed_ms: Option<u128> = None;
    let mut accumulated_ms = 0.0;

    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let columns = split_csv_line(&line);
        if let Some(frametime_ms) = frametime_idx
            .and_then(|idx| columns.get(idx))
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
        {
            let raw_elapsed = elapsed_idx
                .and_then(|idx| columns.get(idx))
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite());

            let elapsed_ms = if let Some(raw) = raw_elapsed {
                let val = raw.max(0.0) as u128;
                if first_elapsed_ms.is_none() {
                    first_elapsed_ms = Some(val);
                }
                val.saturating_sub(first_elapsed_ms.unwrap_or(0))
            } else {
                let val = accumulated_ms as u128;
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

    #[test]
    fn parses_header_based_frametime_csv() {
        let header = "elapsed_ms,frametime_ms";
        let data = "10,16.7\n20,33.4\n";
        let events = parse_frame_events(header, data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized
        assert_eq!(events[1].elapsed_ms, 10); // 20 - 10
        assert_eq!(events[1].frametime_ms, 33.4);
    }

    #[test]
    fn parses_quoted_csv_fields() {
        let header = "elapsed_ms,\"frame,time\",frametime_ms";
        let data = "10,\"ignored, value\",16.7\n";
        let events = parse_frame_events(header, data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized
        assert_eq!(events[0].frametime_ms, 16.7);
    }

    #[test]
    fn skips_non_finite_frametimes() {
        let header = "elapsed_ms,frametime_ms";
        let data = "10,NaN\n20,inf\n30,16.7\n";
        let events = parse_frame_events(header, data.lines().map(|s| Ok(s.to_owned()))).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_ms, 0); // Normalized (30 - 30)
        assert_eq!(events[0].frametime_ms, 16.7);
    }

    #[test]
    fn read_frame_events_respects_newline_boundary_offset() -> anyhow::Result<()> {
        use std::io::Write;
        let temp_dir = std::env::temp_dir().join(format!("stutter_test_mangohud_{}", std::process::id()));
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
        let events = read_frame_events(&path, 0)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frametime_ms, 16.7);

        // Case 2: ignore_offset = offset_after_header.
        // offset_after_header-1 is '\n'.
        // Should NOT skip the first line (row1).
        let events = read_frame_events(&path, offset_after_header)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frametime_ms, 16.7);

        // Case 3: ignore_offset = offset_after_header + 2 (mid row1).
        // Should skip partial row1, read row2.
        let events = read_frame_events(&path, offset_after_header + 2)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frametime_ms, 33.4);

        // Case 4: ignore_offset = offset_after_row1.
        // offset_after_row1-1 is '\n'.
        // Should NOT skip row2.
        let events = read_frame_events(&path, offset_after_row1)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frametime_ms, 33.4);

        fs::remove_dir_all(temp_dir).ok();
        Ok(())
    }
}
