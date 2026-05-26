use super::{parser::MangoHudLiveParser, schema::detect_layout, *};
use crate::recorder::FrameEvent;

pub async fn tail_frames(
    path: std::path::PathBuf,
    start_offset: u64,
    tx: tokio::sync::mpsc::Sender<FrameEvent>,
    idle_sleep: Duration,
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
                tokio::time::sleep(idle_sleep).await;
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
                && tx.is_closed()
            {
                return Ok(());
            }
        }
    }
}
