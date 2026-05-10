use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PsiSnapshot {
    pub cpu_some_avg10: f64,
    pub cpu_some_total_us: u64,
    pub mem_some_avg10: f64,
    pub mem_some_total_us: u64,
    pub mem_full_avg10: f64,
    pub mem_full_total_us: u64,
    pub io_some_avg10: f64,
    pub io_some_total_us: u64,
    pub io_full_avg10: f64,
    pub io_full_total_us: u64,
}

pub struct PsiReader {
    proc_root: PathBuf,
}

impl PsiReader {
    pub fn new() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
        }
    }

    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { proc_root: root }
    }

    pub fn read(&self) -> anyhow::Result<PsiSnapshot> {
        let mut snapshot = PsiSnapshot::default();

        if let Ok(cpu) = self.read_file("pressure/cpu")
            && let Some(some) = cpu.get("some")
        {
            snapshot.cpu_some_avg10 = some.avg10;
            snapshot.cpu_some_total_us = some.total;
        }

        if let Ok(mem) = self.read_file("pressure/memory") {
            if let Some(some) = mem.get("some") {
                snapshot.mem_some_avg10 = some.avg10;
                snapshot.mem_some_total_us = some.total;
            }
            if let Some(full) = mem.get("full") {
                snapshot.mem_full_avg10 = full.avg10;
                snapshot.mem_full_total_us = full.total;
            }
        }

        if let Ok(io) = self.read_file("pressure/io") {
            if let Some(some) = io.get("some") {
                snapshot.io_some_avg10 = some.avg10;
                snapshot.io_some_total_us = some.total;
            }
            if let Some(full) = io.get("full") {
                snapshot.io_full_avg10 = full.avg10;
                snapshot.io_full_total_us = full.total;
            }
        }

        Ok(snapshot)
    }

    fn read_file(&self, rel_path: &str) -> anyhow::Result<HashMap<String, PsiLine>> {
        let content = fs::read_to_string(self.proc_root.join(rel_path))?;
        let mut results = HashMap::new();

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                log::debug!("psi_line_too_short path={} line={:?}", rel_path, line);
                continue;
            }

            let kind = parts[0].to_owned();
            let mut avg10 = 0.0;
            let mut total = 0;

            for part in &parts[1..] {
                if let Some(val) = part.strip_prefix("avg10=") {
                    avg10 = val.parse().unwrap_or(0.0);
                } else if let Some(val) = part.strip_prefix("total=") {
                    total = val.parse().unwrap_or(0);
                }
            }

            results.insert(kind, PsiLine { avg10, total });
        }

        Ok(results)
    }
}

impl Default for PsiReader {
    fn default() -> Self {
        Self::new()
    }
}

struct PsiLine {
    avg10: f64,
    total: u64,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    #[test]
    fn parses_psi_files() {
        let dir = std::env::temp_dir().join(format!("stutter-psi-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pressure_dir = dir.join("pressure");
        fs::create_dir_all(&pressure_dir).unwrap();

        fs::write(
            pressure_dir.join("cpu"),
            "some avg10=1.23 avg60=4.56 avg300=7.89 total=1000\n",
        )
        .unwrap();

        fs::write(
            pressure_dir.join("memory"),
            "some avg10=0.10 avg60=0.20 avg300=0.30 total=2000\nfull avg10=0.05 avg60=0.06 avg300=0.07 total=500\n",
        )
        .unwrap();

        fs::write(
            pressure_dir.join("io"),
            "some avg10=10.00 avg60=11.00 avg300=12.00 total=3000\nfull avg10=5.00 avg60=6.00 avg300=7.00 total=1500\n",
        )
        .unwrap();

        let reader = PsiReader::with_root(dir.clone());
        let snapshot = reader.read().unwrap();

        assert_eq!(snapshot.cpu_some_avg10, 1.23);
        assert_eq!(snapshot.cpu_some_total_us, 1000);
        assert_eq!(snapshot.mem_some_avg10, 0.10);
        assert_eq!(snapshot.mem_some_total_us, 2000);
        assert_eq!(snapshot.mem_full_avg10, 0.05);
        assert_eq!(snapshot.mem_full_total_us, 500);
        assert_eq!(snapshot.io_some_avg10, 10.00);
        assert_eq!(snapshot.io_some_total_us, 3000);
        assert_eq!(snapshot.io_full_avg10, 5.00);
        assert_eq!(snapshot.io_full_total_us, 1500);
    }
}
