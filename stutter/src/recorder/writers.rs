use std::{fs, io, io::Write, path::PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::metrics::IntervalRecord as MetricsIntervalRecord;

#[derive(Debug)]
pub struct NdjsonWriter {
    file: fs::File,
    wrote_any: bool,
    finished: bool,
    path: PathBuf,
}

pub enum CsvOutput {
    File(io::BufWriter<fs::File>),
    Stdout(io::BufWriter<io::Stdout>),
}

pub struct IntervalCsvWriter {
    output: CsvOutput,
    path_label: String,
    finished: bool,
}

impl std::fmt::Debug for IntervalCsvWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntervalCsvWriter")
            .field("path", &self.path_label)
            .field("finished", &self.finished)
            .finish()
    }
}

impl IntervalCsvWriter {
    pub fn create_file(path: PathBuf) -> anyhow::Result<Self> {
        if path.file_name().is_none() {
            anyhow::bail!("CSV destination has no file name: {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(&path)
            .with_context(|| format!("failed to create interval CSV {}", path.display()))?;
        write_interval_csv_header(&mut file)?;
        Ok(Self {
            output: CsvOutput::File(io::BufWriter::new(file)),
            path_label: path.display().to_string(),
            finished: false,
        })
    }

    pub fn stdout() -> Self {
        let mut stdout = io::stdout();
        let _ = write_interval_csv_header(&mut stdout);
        Self {
            output: CsvOutput::Stdout(io::BufWriter::new(stdout)),
            path_label: "stdout".to_owned(),
            finished: false,
        }
    }

    pub fn push(&mut self, record: &MetricsIntervalRecord) -> anyhow::Result<()> {
        match &mut self.output {
            CsvOutput::File(writer) => write_interval_csv_row(writer, record),
            CsvOutput::Stdout(writer) => write_interval_csv_row(writer, record),
        }
        .with_context(|| format!("failed to write interval CSV {}", self.path_label))
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }
        match &mut self.output {
            CsvOutput::File(writer) => {
                writer.flush()?;
                writer.get_ref().sync_all()?;
            }
            CsvOutput::Stdout(writer) => {
                writer.flush()?;
            }
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for IntervalCsvWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "interval_csv_finish_failed path={} err={err:#}",
                self.path_label
            );
        }
    }
}

impl NdjsonWriter {
    pub fn create(path: PathBuf) -> anyhow::Result<Self> {
        if path.file_name().is_none() {
            anyhow::bail!(
                "NDJSON stream destination has no file name: {}",
                path.display()
            );
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = fs::File::create(&path)
            .with_context(|| format!("failed to create NDJSON stream {}", path.display()))?;

        Ok(Self {
            file,
            wrote_any: false,
            finished: false,
            path,
        })
    }

    pub fn push<T: Serialize>(&mut self, value: &T) -> anyhow::Result<()> {
        if self.finished {
            anyhow::bail!("NDJSON stream {} is already finalized", self.path.display());
        }

        serde_json::to_writer(&mut self.file, value)
            .with_context(|| format!("failed to write NDJSON stream {}", self.path.display()))?;
        self.file.write_all(b"\n")?;
        self.wrote_any = true;
        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }

        self.file
            .sync_all()
            .with_context(|| format!("failed to sync NDJSON stream {}", self.path.display()))?;
        self.finished = true;
        Ok(())
    }
}

pub struct StdoutJsonStream {
    stdout: std::io::Stdout,
}

impl StdoutJsonStream {
    pub fn new() -> Self {
        Self {
            stdout: std::io::stdout(),
        }
    }

    pub fn push<T: serde::Serialize>(&mut self, value: &T) -> anyhow::Result<()> {
        super::write_ndjson_value(&mut self.stdout, value)
    }
}

impl Default for StdoutJsonStream {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StdoutJsonStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdoutJsonStream").finish()
    }
}

pub fn write_ndjson_value<W, T>(writer: &mut W, value: &T) -> anyhow::Result<()>
where
    W: std::io::Write,
    T: serde::Serialize,
{
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

impl Drop for NdjsonWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "ndjson_finish_failed path={} err={err:#}",
                self.path.display()
            );
        }
    }
}

fn write_interval_csv_header(file: &mut dyn io::Write) -> io::Result<()> {
    writeln!(
        file,
        "elapsed_ms,task,active,class,comm,process_pid,process_comm,samples,stored_samples,truncated_samples,min_ns,avg_ns,p95_ns,p99_ns,max_ns,over_1ms,over_2ms,over_5ms,busiest_cpu,busiest_cpu_samples,worst_cpu,worst_cpu_max_ns,spikiest_cpu,spikiest_cpu_spikes,percentile_scope,major_faults,minor_faults,cpu_psi_some,mem_psi_some,mem_psi_full,io_psi_some,io_psi_full,cumulative_drop_counters_total,cpu_cycles,cpu_instructions,cpu_ipc,cache_references,cache_misses,cache_miss_rate,cache_mpki,cpu_perf_multiplexed,cpu_perf_scaled,cpu_perf_unavailable_reason"
    )
}

fn write_interval_csv_row(
    file: &mut dyn io::Write,
    record: &MetricsIntervalRecord,
) -> io::Result<()> {
    let cpu_perf = record.cpu_perf.as_ref();
    write!(file, "{},", record.elapsed_ms)?;
    write!(file, "{},", record.task)?;
    write!(file, "{},", record.active)?;
    write!(file, "{},", record.class)?;
    write!(file, "{},", csv_escape(&record.comm))?;
    write!(file, "{},", option_u32(record.process_pid))?;
    write!(file, "{},", csv_escape(&record.process_comm))?;
    write!(file, "{},", record.samples)?;
    write!(file, "{},", record.stored_samples)?;
    write!(file, "{},", record.truncated_samples)?;
    write!(file, "{},", record.min_ns)?;
    write!(file, "{},", record.avg_ns)?;
    write!(file, "{},", record.p95_ns)?;
    write!(file, "{},", record.p99_ns)?;
    write!(file, "{},", record.max_ns)?;
    write!(file, "{},", record.over_1ms)?;
    write!(file, "{},", record.over_2ms)?;
    write!(file, "{},", record.over_5ms)?;
    write!(file, "{},", option_u32(record.busiest_cpu))?;
    write!(file, "{},", record.busiest_cpu_samples)?;
    write!(file, "{},", option_u32(record.worst_cpu))?;
    write!(file, "{},", record.worst_cpu_max_ns)?;
    write!(file, "{},", option_u32(record.spikiest_cpu))?;
    write!(file, "{},", record.spikiest_cpu_spikes)?;
    write!(file, "{},", csv_escape(&record.percentile_scope))?;
    write!(file, "{},", record.major_faults)?;
    write!(file, "{},", record.minor_faults)?;
    write!(file, "{},", record.cpu_psi_some)?;
    write!(file, "{},", record.mem_psi_some)?;
    write!(file, "{},", record.mem_psi_full)?;
    write!(file, "{},", record.io_psi_some)?;
    write!(file, "{},", record.io_psi_full)?;
    write!(file, "{},", record.drop_counters.total())?;
    write!(
        file,
        "{},",
        option_u64(cpu_perf.and_then(|perf| perf.cycles))
    )?;
    write!(
        file,
        "{},",
        option_u64(cpu_perf.and_then(|perf| perf.instructions))
    )?;
    write!(file, "{},", option_f64(cpu_perf.and_then(|perf| perf.ipc)))?;
    write!(
        file,
        "{},",
        option_u64(cpu_perf.and_then(|perf| perf.cache_references))
    )?;
    write!(
        file,
        "{},",
        option_u64(cpu_perf.and_then(|perf| perf.cache_misses))
    )?;
    write!(
        file,
        "{},",
        option_f64(cpu_perf.and_then(|perf| perf.cache_miss_rate))
    )?;
    write!(
        file,
        "{},",
        option_f64(cpu_perf.and_then(|perf| perf.cache_mpki))
    )?;
    write!(
        file,
        "{},",
        option_bool(cpu_perf.map(|perf| perf.multiplexed))
    )?;
    write!(file, "{},", option_bool(cpu_perf.map(|perf| perf.scaled)))?;
    writeln!(
        file,
        "{}",
        csv_escape(
            cpu_perf
                .and_then(|perf| perf.unavailable_reason.as_deref())
                .unwrap_or("")
        )
    )
}

fn option_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_u64(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_f64(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.6}"))
        .unwrap_or_default()
}

fn option_bool(value: Option<bool>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
pub fn write_interval_csv(
    path: &std::path::Path,
    interval_records: &[MetricsIntervalRecord],
) -> anyhow::Result<()> {
    let mut writer = IntervalCsvWriter::create_file(path.to_path_buf())?;

    for record in interval_records {
        writer.push(record)?;
    }

    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    #[test]
    fn ndjson_writer_outputs_valid_stream() {
        let dir = temp_dir("ndjson-writer");
        fs::create_dir_all(&dir).unwrap();
        let empty_path = dir.join("empty.json");
        {
            let mut writer = NdjsonWriter::create(empty_path.clone()).unwrap();
            writer.finish().unwrap();
        }
        assert!(fs::read_to_string(&empty_path).unwrap().is_empty());

        let single_path = dir.join("single.json");
        {
            let mut writer = NdjsonWriter::create(single_path.clone()).unwrap();
            writer.push(&serde_json::json!({"one": true})).unwrap();
            writer.finish().unwrap();
        }
        let single: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&single_path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(single.len() as u64, 1);

        let path = dir.join("items.json");

        {
            let mut writer = NdjsonWriter::create(path.clone()).unwrap();
            writer.push(&serde_json::json!({"a": 1})).unwrap();
            writer.push(&serde_json::json!({"b": 2})).unwrap();
            writer.finish().unwrap();
        }

        let values: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(values.len() as u64, 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_array_writer_rejects_path_without_file_name() {
        let err = NdjsonWriter::create(PathBuf::from("/")).unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn interval_csv_writer_streams_header_and_rows() {
        let dir = temp_dir("interval-csv-writer");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interval.csv");

        {
            let mut writer = IntervalCsvWriter::create_file(path.clone()).unwrap();
            writer.push(&test_interval_record()).unwrap();
            writer.finish().unwrap();
        }

        let csv = fs::read_to_string(&path).unwrap();
        assert!(csv.starts_with("elapsed_ms,task,active"));
        assert!(csv.contains("worker"));
        fs::remove_dir_all(dir).ok();
    }

    fn test_interval_record() -> MetricsIntervalRecord {
        MetricsIntervalRecord {
            elapsed_ms: 1,
            task: 2,
            active: true,
            class: TaskClass::Game,
            comm: "worker".to_owned(),
            process_pid: Some(2),
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            min_ns: 1,
            avg_ns: 1,
            p95_ns: 1,
            p99_ns: 1,
            major_faults: 0,
            minor_faults: 0,
            max_ns: 1,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            cpu_psi_some: 0.0,
            mem_psi_some: 0.0,
            mem_psi_full: 0.0,
            io_psi_some: 0.0,
            io_psi_full: 0.0,
            percentile_scope: "all".to_owned(),
            histogram: Vec::new(),
            drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
            ..Default::default()
        }
    }

    #[test]
    fn test_write_ndjson_value() {
        let event = crate::recorder::SpikeEvent {
            elapsed_ms: Some(100),
            task: 123,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(123),
            process_comm: "game".into(),
            comm: "game".to_owned(),
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 2000,
            switch_ns: 3000,
            target_pending_wakeups: 0,
            major_faults: 1,
            minor_faults: 2,
            ..Default::default()
        };

        let mut buf = Vec::new();
        write_ndjson_value(&mut buf, &event).unwrap();
        write_ndjson_value(&mut buf, &event).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 2);

        for line in lines {
            let decoded: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(decoded.is_object());
            assert_eq!(decoded["task"], 123);
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
