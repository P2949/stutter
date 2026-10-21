use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};

use super::{WorkloadProgress, WorkloadProgressRecord};

/// Read only newly appended markers; historical input never becomes fresh work.
pub struct WorkloadProgressReader {
    path: PathBuf,
    reader: BufReader<File>,
    identity: (u64, u64),
    pending: Vec<u8>,
}

impl WorkloadProgressReader {
    pub fn open_tail(path: &Path) -> anyhow::Result<Self> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .with_context(|| format!("open workload progress log {}", path.display()))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("workload progress input must be a regular file");
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            path: path.to_owned(),
            reader: BufReader::new(file),
            identity: (metadata.dev(), metadata.ino()),
            pending: Vec::new(),
        })
    }

    pub fn poll(
        &mut self,
        monitor_start_ns: u64,
        now_ns: u64,
    ) -> anyhow::Result<Vec<WorkloadProgressRecord>> {
        let metadata = fs::metadata(&self.path)?;
        if (metadata.dev(), metadata.ino()) != self.identity
            || metadata.len() < self.reader.stream_position()?
        {
            bail!("workload progress log was replaced or truncated");
        }
        let mut records = Vec::new();
        let mut processed = 0;
        while processed < 128 {
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                break;
            }
            let count = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .unwrap_or(available.len());
            if self.pending.len() + count > 65_536 {
                bail!("workload progress marker exceeds 64 KiB");
            }
            self.pending.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if self.pending.last() != Some(&b'\n') {
                continue;
            }
            let line = std::mem::take(&mut self.pending);
            processed += 1;
            let progress: WorkloadProgress =
                serde_json::from_slice(&line).context("malformed workload progress marker")?;
            let observed_now = now_ns.max(crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?);
            if progress.monotonic_ns > observed_now {
                bail!("workload progress timestamp is in the future");
            }
            let Some(delta) = progress.monotonic_ns.checked_sub(monitor_start_ns) else {
                continue;
            };
            records.push(WorkloadProgressRecord {
                elapsed_ms: delta / 1_000_000,
                progress,
            });
        }
        Ok(records)
    }
}
