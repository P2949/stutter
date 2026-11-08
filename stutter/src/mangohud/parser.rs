use std::io::BufRead;

#[cfg(test)]
use super::schema::schema_from_headers;
use super::{
    plausibility::MangoHudFramePlausibilityFilter,
    schema::{MangoHudCsvSchema, detect_layout, elapsed_value_to_ms, split_csv_line},
    *,
};
use crate::recorder::FrameEvent;

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
pub(super) fn parse_frame_events<I>(
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
    let mut plausibility_filter = MangoHudFramePlausibilityFilter::default();

    let alignment_recorder_elapsed_ms = alignment_monotonic_ns
        .zip(recorder_start_monotonic_ns)
        .map(|(m, r)| m.saturating_sub(r) / 1_000_000);

    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some((raw_val, frametime_ms)) = parse_frame_line(schema, &line) {
            if !plausibility_filter.accept(raw_val, frametime_ms) {
                continue;
            }

            if first_elapsed_ms.is_none() {
                first_elapsed_ms = raw_val;
            }

            let elapsed_ms = aligned_elapsed_ms(
                raw_val,
                first_elapsed_ms,
                &mut accumulated_ms,
                frametime_ms,
                alignment_raw_elapsed_ms,
                alignment_recorder_elapsed_ms,
            );

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
    plausibility_filter: MangoHudFramePlausibilityFilter,
}

/// What one CSV line was. A blank or comment line is not data and not loss; a line that was data
/// but could not be used is a rejected row, which is loss.
#[derive(Clone, Debug)]
pub enum LineOutcome {
    Frame(FrameEvent),
    /// Blank or a `#` comment.
    NotData,
    /// A data row that did not parse or failed the plausibility filter.
    Rejected,
    /// A well-formed row with no source timestamp. Without the source's own event time a frame
    /// cannot be placed on any clock, so it is loss, never an assumed time zero.
    NoTimestamp,
}

impl MangoHudLiveParser {
    pub(super) fn new(schema: MangoHudCsvSchema) -> Self {
        Self {
            schema,
            plausibility_filter: MangoHudFramePlausibilityFilter::default(),
        }
    }

    pub fn parse_line(&mut self, line: &str) -> Option<FrameEvent> {
        match self.parse_line_checked(line) {
            LineOutcome::Frame(event) => Some(event),
            LineOutcome::NotData | LineOutcome::Rejected | LineOutcome::NoTimestamp => None,
        }
    }

    /// Like [`parse_line`](Self::parse_line) but says *why* there is no frame, so a rejected row
    /// is counted as loss and a blank or comment line is not.
    pub fn parse_line_checked(&mut self, line: &str) -> LineOutcome {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return LineOutcome::NotData;
        }
        let Some((raw_elapsed_ms, frametime_ms)) = parse_frame_line(&self.schema, line) else {
            return LineOutcome::Rejected;
        };
        if !self
            .plausibility_filter
            .accept(raw_elapsed_ms, frametime_ms)
        {
            return LineOutcome::Rejected;
        }
        let Some(elapsed_ms) = raw_elapsed_ms else {
            return LineOutcome::NoTimestamp;
        };
        LineOutcome::Frame(FrameEvent {
            elapsed_ms,
            frametime_ms,
        })
    }
}

pub(super) fn parse_frame_line(
    schema: &MangoHudCsvSchema,
    line: &str,
) -> Option<(Option<u64>, f64)> {
    let columns = split_csv_line(line);
    let frametime_ms = columns
        .get(schema.frametime_idx)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)?;

    let elapsed_ms = schema
        .elapsed_idx
        .and_then(|idx| columns.get(idx))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|raw| elapsed_value_to_ms(raw, schema.elapsed_unit));

    Some((elapsed_ms, frametime_ms))
}

fn aligned_elapsed_ms(
    raw_val: Option<u64>,
    first_elapsed_ms: Option<u64>,
    accumulated_ms: &mut f64,
    frametime_ms: f64,
    alignment_raw_elapsed_ms: Option<u64>,
    alignment_recorder_elapsed_ms: Option<u64>,
) -> u64 {
    if let (Some(raw), Some(first_raw), Some(observed_ms)) = (
        raw_val,
        alignment_raw_elapsed_ms,
        alignment_recorder_elapsed_ms,
    ) {
        observed_ms + raw.saturating_sub(first_raw)
    } else if let Some(raw) = raw_val {
        let first = first_elapsed_ms.unwrap_or(0);
        raw.saturating_sub(first)
    } else if let Some(observed_ms) = alignment_recorder_elapsed_ms {
        let val = observed_ms + *accumulated_ms as u64;
        *accumulated_ms += frametime_ms;
        val
    } else {
        let val = *accumulated_ms as u64;
        *accumulated_ms += frametime_ms;
        val
    }
}
