use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

pub(super) fn load_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("failed to parse {}", path.display()))
}

pub(super) fn load_ndjson_file_filtered<T: DeserializeOwned, F: Fn(&T) -> bool>(
    path: &Path,
    filter: F,
) -> Result<Vec<T>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let iter = deserializer.into_iter::<T>();
    for item in iter {
        let val = item.with_context(|| format!("failed to parse {}", path.display()))?;
        if filter(&val) {
            results.push(val);
        }
    }
    Ok(results)
}

pub(super) fn count_ndjson_file<T: DeserializeOwned>(path: &Path) -> Result<usize> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let iter = deserializer.into_iter::<T>();
    let mut count = 0usize;
    for item in iter {
        item.with_context(|| format!("failed to parse {}", path.display()))?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::recorder::FrameEvent;

    #[test]
    #[ignore = "benchmark"]
    fn bench_report_load_large_ndjson() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("large_bench.ndjson");
        let mut data = String::with_capacity(100_000 * 50);
        for i in 0..100_000 {
            data.push_str(&format!(
                r#"{{"elapsed_ms":{},"frametime_ms":16.7}}"#,
                i * 16
            ));
            data.push('\n');
        }
        std::fs::write(&path, data).unwrap();

        let start = Instant::now();
        let events = load_ndjson_file_filtered::<FrameEvent, _>(&path, |_| true).unwrap();
        let duration = start.elapsed();

        assert_eq!(events.len(), 100_000);
        println!("Parsed 100k NDJSON rows in {:?}", duration);
        std::fs::remove_file(path).ok();
    }
}
