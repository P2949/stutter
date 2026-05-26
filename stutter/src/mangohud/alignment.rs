use std::io::{BufRead, Read, Seek, SeekFrom};

use tokio::time::sleep;

use super::{
    parser::parse_frame_line,
    plausibility::MangoHudFramePlausibilityFilter,
    schema::{MangoHudCsvLayout, try_detect_layout},
    *,
};
use crate::recorder::monotonic_now_ns;

pub async fn poll_alignment(
    path: &Path,
    start_offset: u64,
    poll_interval: Duration,
) -> anyhow::Result<(u64, u64)> {
    let mut layout_cache: Option<MangoHudCsvLayout> = None;

    loop {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                sleep(poll_interval).await;
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        let len = file.metadata()?.len();

        if layout_cache.is_none() {
            match try_detect_layout(path) {
                Ok(Some(layout)) => layout_cache = Some(layout),
                Ok(None) => {
                    sleep(poll_interval).await;
                    continue;
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    sleep(poll_interval).await;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }

        let Some(layout) = layout_cache.as_ref() else {
            sleep(poll_interval).await;
            continue;
        };
        let read_offset = start_offset.max(layout.data_start_offset);

        if len > read_offset
            && let Some(alignment) =
                read_first_plausible_alignment(path, &mut file, layout, read_offset)?
        {
            return Ok(alignment);
        }

        sleep(poll_interval).await;
    }
}

fn read_first_plausible_alignment(
    path: &Path,
    file: &mut fs::File,
    layout: &MangoHudCsvLayout,
    read_offset: u64,
) -> anyhow::Result<Option<(u64, u64)>> {
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

    let mut plausibility_filter = MangoHudFramePlausibilityFilter::default();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }

        if let Some((raw_elapsed_ms, frametime_ms)) = parse_frame_line(&layout.schema, &line) {
            if !plausibility_filter.accept(raw_elapsed_ms, frametime_ms) {
                continue;
            }

            let observed_ns = monotonic_now_ns().unwrap_or(0);
            return Ok(Some((raw_elapsed_ms.unwrap_or(0), observed_ns)));
        }
    }
}
