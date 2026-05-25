use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Context, Result};

const MAX_FILE_SIZE: usize = 1000;

#[derive(Debug)]
struct BaselineEntry {
    old_loc: usize,
    path: String,
}

pub fn run_maturity_report(root: &Path) -> Result<()> {
    let baseline_path = root.join("docs/internal/cleanup-baseline.md");
    let content = fs::read_to_string(&baseline_path)
        .context("Failed to read cleanup-baseline.md")?;
    
    let mut baseline = Vec::new();
    let mut parsing_table = false;
    
    for line in content.lines() {
        if line.starts_with("| LOC") || line.starts_with("|------") {
            parsing_table = true;
            continue;
        }
        if parsing_table && line.starts_with('|') {
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 3 {
                if let Ok(loc) = parts[1].parse::<usize>() {
                    let path = parts[2].to_string();
                    baseline.push(BaselineEntry { old_loc: loc, path });
                }
            }
        }
    }

    println!("--- MATURITY REPORT ---");
    let mut baseline_paths = std::collections::HashSet::new();

    for entry in &baseline {
        let full_path = root.join(entry.path.trim_start_matches("./"));
        baseline_paths.insert(full_path.clone());
        
        let new_loc = if full_path.exists() {
            fs::read_to_string(&full_path)
                .map(|c| c.lines().count())
                .unwrap_or(0)
        } else {
            0
        };
        
        let diff = new_loc as isize - entry.old_loc as isize;
        let percent = if entry.old_loc > 0 {
            (diff as f64 / entry.old_loc as f64) * 100.0
        } else {
            0.0
        };
        
        println!("{}: {} -> {} ({:+.1}%)", entry.path, entry.old_loc, new_loc, percent);
    }
    
    println!("\n--- ARCHITECTURE GATES ---");
    let mut oversized = Vec::new();
    find_oversized_rs_files(root, root, &baseline_paths, &mut oversized);
    
    if !oversized.is_empty() {
        println!("WARNING: Found new or unbaselined files exceeding {} LOC:", MAX_FILE_SIZE);
        for (path, loc) in oversized {
            println!("  {}: {} LOC", path.display(), loc);
        }
    } else {
        println!("OK: No new files exceed {} LOC.", MAX_FILE_SIZE);
    }
    
    Ok(())
}

fn find_oversized_rs_files(root: &Path, dir: &Path, baseline: &std::collections::HashSet<PathBuf>, oversized: &mut Vec<(PathBuf, usize)>) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() && name != "target" && name != ".git" {
            find_oversized_rs_files(root, &path, baseline, oversized);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if !baseline.contains(&path) {
                if let Ok(content) = fs::read_to_string(&path) {
                    let loc = content.lines().count();
                    if loc > MAX_FILE_SIZE {
                        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                        oversized.push((rel, loc));
                    }
                }
            }
        }
    }
}
